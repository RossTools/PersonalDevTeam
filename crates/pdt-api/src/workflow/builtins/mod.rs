//! Built-in workflows (Phase 7a spec §8): the reactions that used to be hard-coded, installed host-wide when pdtd
//! starts. A built-in the user never edited follows newer templates; one the user edited is left alone. They can be
//! disabled, not deleted.

mod base_moved;
mod comments_to_agent;
mod cut_release;
mod finish_merged;
mod github_issue_done;
mod late_commits;
mod missing_release;
mod pick_up_tickets;
mod pr_feedback;
mod pr_notices;

pub(crate) use cut_release::cut_release_template;
pub(crate) use missing_release::missing_release_template;

use crate::routes::{AppState, now_ms};
use pdt_core::workflow::api::{Author, Scope, Source, Workflow};
use pdt_core::workflow::doc::WorkflowDoc;
use pdt_core::workflow::template::Category;
use serde_json::json;

#[derive(Clone, Copy)]
pub struct Builtin {
    pub id: &'static str,
    /// Enabled when first installed.
    pub enabled: bool,
    /// Its group in the New workflow gallery (PDT-229).
    pub category: Category,
    pub doc: fn() -> WorkflowDoc,
}

pub const PR_NOTICES: &str = "pr-notices";
pub const FINISH_MERGED: &str = "finish-merged";
pub const COMMENTS_TO_AGENT: &str = "comments-to-agent";
pub const BASE_MOVED: &str = "base-moved";
pub const PR_FEEDBACK: &str = "pr-feedback";
pub const LATE_COMMITS: &str = "late-commits";
pub const GITHUB_ISSUE_DONE: &str = "github-issue-done";
pub const PICK_UP_TICKETS: &str = "pick-up-tickets";
/// The Cut a release template (without `builtin:`); a built-in workflow until PDT-301 ([`RETIRED`]).
pub const CUT_RELEASE: &str = "cut-release";

/// PDT's built-ins, one module each (its document and prompts), in install order: one line per built-in.
pub const BUILTINS: &[Builtin] = &[
    Builtin { id: PICK_UP_TICKETS, enabled: true, category: Category::Tickets, doc: pick_up_tickets::pick_up_tickets },
    Builtin { id: PR_NOTICES, enabled: true, category: Category::CiPr, doc: pr_notices::pr_notices },
    Builtin { id: FINISH_MERGED, enabled: true, category: Category::Tickets, doc: finish_merged::finish_merged },
    Builtin { id: LATE_COMMITS, enabled: true, category: Category::CiPr, doc: late_commits::late_commits },
    Builtin {
        id: COMMENTS_TO_AGENT,
        enabled: true,
        category: Category::Tickets,
        doc: comments_to_agent::comments_to_agent,
    },
    Builtin { id: BASE_MOVED, enabled: false, category: Category::CiPr, doc: base_moved::base_moved },
    Builtin { id: PR_FEEDBACK, enabled: false, category: Category::CiPr, doc: pr_feedback::pr_feedback },
];

/// The GitHub plugin's built-ins ([`crate::integrations::Integration::builtin_workflows`]): left out while it is off
/// (PDT-251).
pub const GITHUB_BUILTINS: &[Builtin] = &[Builtin {
    id: GITHUB_ISSUE_DONE,
    enabled: true,
    category: Category::Issues,
    doc: github_issue_done::github_issue_done,
}];

/// PDT's built-ins, then the enabled integrations' ([`crate::integrations::Integration::builtin_workflows`]).
pub fn all(reg: &crate::integrations::Registry) -> Vec<Builtin> {
    BUILTINS.iter().copied().chain(reg.builtin_workflows()).collect()
}

/// Whether a stored workflow is the built-in of an integration that is turned off (or not in this build; PDT-173): an
/// install from before integrations could be turned off has Jira's, enabled. It can't run (its kinds aren't
/// registered) nor be deleted, so it is left out of the workflow list and the engine until its integration is on
/// again, when it comes back as it was.
pub fn hidden(reg: &crate::integrations::Registry, w: &Workflow) -> bool {
    w.builtin.as_deref().is_some_and(|id| !all(reg).iter().any(|b| b.id == id))
}

fn doc(v: serde_json::Value) -> WorkflowDoc {
    serde_json::from_value(v).expect("a built-in workflow is a valid document")
}

/// The id of the Report a missing scheduled release template (without `builtin:`).
pub const MISSING_RELEASE: &str = "missing-scheduled-release";

/// Installs or updates the built-ins (once per pdtd; callers hold the engine lock).
pub(crate) async fn ensure_installed(st: &AppState) {
    use std::sync::atomic::Ordering;
    if st.builtins_installed.load(Ordering::SeqCst) {
        return;
    }
    let mut ok = true;
    if let Err(e) = detach_retired(st).await {
        ok = false;
        tracing::warn!(error = %e, "could not keep retired built-in workflows as the user's own");
    }
    for b in &all(&st.registry()) {
        if let Err(e) = install(st, b).await {
            ok = false;
            tracing::warn!(builtin = b.id, error = %e, "could not install a built-in workflow");
        }
    }
    if let Err(e) = super::templates::follow_templates(st).await {
        ok = false;
        tracing::warn!(error = %e, "could not move workflows to their templates' newer versions");
    }
    // Tried again on the next tick when something failed.
    st.builtins_installed.store(ok, Ordering::SeqCst);
}

/// Built-ins PDT no longer ships (PDT-295): they automated PDT's own development (bug triage, outside pull request
/// review, nightly release watches), and PDT never special-cases its own repository; Cut a release is a template
/// since PDT-301 (made per project with its release workflow). A host that has one keeps it as its own workflow
/// ([`detach_retired`]); no host gets one installed.
pub(crate) const RETIRED: &[&str] =
    &["pdt-bug-triage", "pdt-pr-review", "pdt-nightly", "pdt-nightly-missing", CUT_RELEASE];

/// Makes each installed [`RETIRED`] built-in an ordinary workflow of the user's: the same document, enabled state,
/// runs and state, no longer updated by PDT. One the user never edited gets its nodes moved to their kinds' current
/// versions first, as PDT's last update of it (a missing nightly's version 1 watch becomes version 2 with PDT's tag
/// scheme written out); an edited one is left as the user saved it (it runs the same). Repeating it after a failure is
/// harmless.
async fn detach_retired(st: &AppState) -> Result<(), pdt_store::StoreError> {
    let everything = st.integrations.everything();
    let catalog = everything.catalog();
    for id in RETIRED {
        let Some(w) = st.store.workflow_by_builtin(id).await? else { continue };
        let untouched = st.store.current_version_author(&w.id).await? == Some(Author::Builtin);
        // Cut a release came off; one never turned on, edited or run was only ever PDT's: it goes (PDT-301).
        if *id == CUT_RELEASE && untouched && !w.enabled && !ran(st, &w.id).await? {
            st.store.delete_workflow(&w.id).await?;
            tracing::info!(builtin = id, "removed a retired built-in workflow that was never used");
            continue;
        }
        if let Some(mut doc) = st.store.workflow_doc(&w.id, w.version).await?.filter(|_| untouched) {
            let mut moved = false;
            for n in &mut doc.nodes {
                let Some(version) = catalog.spec(&n.kind).map(|s| s.version).filter(|v| *v > n.version) else {
                    continue;
                };
                match catalog.current_config(n) {
                    Ok(config) => {
                        (n.config, n.version, moved) = (config, version, true);
                    }
                    Err(e) => tracing::warn!(workflow = %w.name, node = %n.id, error = %e, "left a node as it was"),
                }
            }
            if moved {
                let note = "kept as your own workflow (PDT no longer ships it)";
                st.store.save_workflow_version(&w.id, &doc, Author::Builtin, note, now_ms()).await?;
            }
        }
        st.store.detach_workflow_builtin(&w.id, now_ms()).await?;
        tracing::info!(builtin = id, workflow = %w.name, "kept a retired built-in workflow as the user's own");
    }
    Ok(())
}

/// Whether a workflow has any run.
async fn ran(st: &AppState, id: &str) -> Result<bool, pdt_store::StoreError> {
    let f = pdt_store::RunFilter { workflow_id: Some(id.into()), limit: 1, ..Default::default() };
    Ok(!st.store.list_workflow_runs(&f).await?.is_empty())
}

async fn install(st: &AppState, b: &Builtin) -> Result<(), pdt_store::StoreError> {
    let template = (b.doc)();
    let now = now_ms();
    match st.store.workflow_by_builtin(b.id).await? {
        None => {
            let enabled = b.enabled;
            let w = Workflow {
                id: uuid::Uuid::now_v7().to_string(),
                name: template.name.clone(),
                description: template.description.clone(),
                scope: Scope::Host,
                project_id: None,
                enabled,
                builtin: Some(b.id.into()),
                source: Source::Host,
                repo_file: None,
                version: 1,
                proposed: None,
                created_at: now,
                updated_at: now,
            };
            st.store.insert_workflow(&w, &template, Author::Builtin, "installed").await?;
            tracing::info!(builtin = b.id, enabled, "installed a built-in workflow");
        }
        Some(w) => {
            let current = st.store.workflow_doc(&w.id, w.version).await?;
            let untouched = st.store.current_version_author(&w.id).await? == Some(Author::Builtin);
            if untouched && current.as_ref() != Some(&template) {
                st.store.save_workflow_version(&w.id, &template, Author::Builtin, "updated by PDT", now).await?;
                tracing::info!(builtin = b.id, "updated a built-in workflow");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_core::workflow::validate::{Severity, validate};

    #[test]
    fn every_builtin_is_valid() {
        for b in &all(crate::integrations::test_registry()) {
            let d = (b.doc)();
            let issues = validate(&d, crate::integrations::test_registry().catalog());
            let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
            assert!(errors.is_empty(), "{}: {errors:?}", b.id);
        }
    }

    /// Cut a release made for a project: a beta channel, a release PR branch.
    fn cut_release_for(given: serde_json::Value) -> WorkflowDoc {
        use pdt_core::workflow::template::{fill, param_values};
        let t = cut_release_template();
        fill(&t.doc, &param_values(&t.params, given.as_object().unwrap()).unwrap()).unwrap()
    }

    /// PDT-194: every port of Cut a release leads somewhere (but a timed wait's event and timeout, which it never
    /// takes), and every node is reached; since PDT-301 it is a template, no built-in.
    #[test]
    fn cut_release_has_no_open_ports() {
        let d = cut_release_for(json!({"workflow": "publish.yml", "releaseBranch": "cut/release", "preName": "Beta",
                                       "preInputs": "{\"channel\": \"beta\"}"}));
        let issues = validate(&d, crate::integrations::test_registry().catalog());
        let open = |i: &&pdt_core::workflow::validate::Issue| {
            i.code == "unreachable" || (i.code == "open_port" && !i.node.as_deref().is_some_and(|n| n.contains("wait")))
        };
        assert!(!issues.iter().any(|i| open(&i)), "{issues:?}");
        assert!(issues.iter().all(|i| i.severity != Severity::Error), "{issues:?}");
        assert!(!all(crate::integrations::test_registry()).iter().any(|b| b.id == CUT_RELEASE), "a template only");
        assert!(RETIRED.contains(&CUT_RELEASE));
    }

    /// PDT-301: Run now asks the kind of release, the other kind (the safe one) first, and a stable one confirms.
    #[test]
    fn cut_release_asks_the_kind_of_release() {
        use crate::workflow::nodes::trigger::ManualConfig;
        let d = cut_release_for(json!({"workflow": "publish.yml", "preName": "Beta"}));
        let manual: ManualConfig = serde_json::from_value(d.node("start").unwrap().config.clone()).unwrap();
        pdt_core::workflow::inputs::check_fields(&manual.inputs, manual.confirm.as_ref()).unwrap();
        assert_eq!(manual.inputs[0].choices[0].label, "Beta");
        let input = |v: serde_json::Value| manual.input(&v).unwrap();
        assert_eq!(input(json!({})), json!({"type": "prerelease"}));
        let stable = input(json!({"type": "stable", "bump": "minor"}));
        assert_eq!(stable, json!({"type": "stable", "bump": "minor"}));
        let ask = |v: &serde_json::Value| pdt_core::workflow::inputs::confirm_text(manual.confirm.as_ref(), v);
        assert_eq!(ask(&stable).as_deref(), Some("This publishes a stable release (minor). Continue?"));
        assert_eq!(ask(&input(json!({}))), None);
        let exact = input(json!({"type": "stable", "version": "2.0.0", "bump": "major"}));
        assert_eq!(exact, json!({"type": "stable", "version": "2.0.0"}), "an exact version, no bump");
        assert!(manual.input(&json!({"channel": "beta"})).unwrap_err().contains("there is no input channel"));
        let stable_first = cut_release_for(json!({"workflow": "publish.yml", "defaultType": "stable"}));
        let m: ManualConfig = serde_json::from_value(stable_first.node("start").unwrap().config.clone()).unwrap();
        assert_eq!(m.input(&json!({})).unwrap()["type"], "stable");
    }

    /// PDT-295: no built-in is a retired one, so none comes back.
    #[test]
    fn no_retired_builtin_is_shipped() {
        for b in &all(crate::integrations::test_registry()) {
            assert!(!RETIRED.contains(&b.id), "{}", b.id);
        }
    }

    /// PDT-294: Report a missing scheduled release, filled in, leaves no port open, and names nothing of PDT's.
    #[test]
    fn the_missing_release_template_fills_in() {
        use pdt_core::workflow::template::{fill, param_values};
        let t = missing_release_template();
        let given = json!({"repo": "acme/app", "workflow": "publish.yml", "tagPattern": "v*-beta.{date}", "hour": 7,
                           "inputs": "{\"channel\": \"beta\"}"});
        let d = fill(&t.doc, &param_values(&t.params, given.as_object().unwrap()).unwrap()).unwrap();
        let issues = validate(&d, crate::integrations::test_registry().catalog());
        assert!(!issues.iter().any(|i| i.code == "unreachable" || i.code == "open_port"), "{issues:?}");
        assert!(issues.iter().all(|i| i.severity != Severity::Error), "{issues:?}");
        let watch = &d.node("watch").unwrap().config;
        assert_eq!(watch["dueBy"], "7:00");
        assert_eq!(watch["onlyIfChanged"], true);
        assert_eq!(d.node("daily").unwrap().config["cron"], "0 7 * * *");
        // The node kind keeps its version 1 id.
        let text = serde_json::to_string(&t).unwrap().to_lowercase().replace("\"github.nightly\"", "");
        for pdt in ["release.yml", "nightly", pdt_core::brand::RELEASE_REPO] {
            assert!(!text.contains(&pdt.to_lowercase()), "names {pdt}");
        }
    }

    /// PDT-294: Cut a release assumes no workflow file, branch, inputs or tag scheme.
    #[test]
    fn cut_release_has_no_pdt_defaults() {
        let text = serde_json::to_string(&cut_release_template()).unwrap().to_lowercase();
        for pdt in ["release.yml", "nightly", "experimental", "channel", "\"release\"", pdt_core::brand::RELEASE_REPO] {
            assert!(!text.contains(&pdt.to_lowercase()), "names {pdt}");
        }
    }

    /// PDT-139: a notice nobody could be told goes on, to the Inbox and the ticket.
    #[test]
    fn pr_notices_leaves_no_notice_untold() {
        let issues = validate(&pr_notices::pr_notices(), crate::integrations::test_registry().catalog());
        let open: Vec<_> =
            issues.iter().filter(|i| i.code == "open_port" && i.node.as_deref() == Some("tell")).collect();
        assert!(open.iter().all(|i| !i.message.contains("untold")), "{open:?}");
        assert!(!issues.iter().any(|i| i.code == "unreachable"), "{issues:?}");
    }
}
