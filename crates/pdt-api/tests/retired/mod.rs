//! The built-ins PDT shipped for its own development until PDT-295 (bug triage, outside pull request review, the
//! nightly release watches), as a host that had them stored them: `fixtures/retired_builtins/<id>.json` is each one's
//! document as its last release installed it. A test puts one in the store before pdtd starts, as an upgraded host
//! has it; pdtd then keeps it as the user's own workflow.
#![allow(dead_code)]

use pdt_core::workflow::api::{Author, Scope, Source, Workflow};
use pdt_core::workflow::doc::WorkflowDoc;
use pdt_store::Store;

pub const BUG_TRIAGE: &str = "pdt-bug-triage";
pub const PR_REVIEW: &str = "pdt-pr-review";
pub const NIGHTLY: &str = "pdt-nightly";
pub const NIGHTLY_MISSING: &str = "pdt-nightly-missing";
pub const ALL: [&str; 4] = [BUG_TRIAGE, PR_REVIEW, NIGHTLY, NIGHTLY_MISSING];

/// The workflow id a retired built-in gets here.
pub fn id(builtin: &str) -> String {
    format!("retired-{builtin}")
}

pub fn doc(builtin: &str) -> WorkflowDoc {
    let text = match builtin {
        BUG_TRIAGE => include_str!("../fixtures/retired_builtins/pdt-bug-triage.json"),
        PR_REVIEW => include_str!("../fixtures/retired_builtins/pdt-pr-review.json"),
        NIGHTLY => include_str!("../fixtures/retired_builtins/pdt-nightly.json"),
        NIGHTLY_MISSING => include_str!("../fixtures/retired_builtins/pdt-nightly-missing.json"),
        other => panic!("no retired built-in {other}"),
    };
    serde_json::from_str(text).unwrap()
}

/// Stores a retired built-in as PDT installed it (host-wide, by PDT, with its built-in id).
pub async fn install(store: &Store, builtin: &str, enabled: bool) -> String {
    let d = doc(builtin);
    let w = Workflow {
        id: id(builtin),
        name: d.name.clone(),
        description: d.description.clone(),
        scope: Scope::Host,
        project_id: None,
        enabled,
        builtin: Some(builtin.into()),
        source: Source::Host,
        repo_file: None,
        version: 1,
        proposed: None,
        created_at: 1,
        updated_at: 1,
    };
    store.insert_workflow(&w, &d, Author::Builtin, "installed").await.unwrap();
    w.id
}
