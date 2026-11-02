//! A built-in Settings sets (PDT-384): a template whose inputs (template params) Settings shows as plain controls. The
//! workflow is the template filled in with them; Settings reads them back from its current document and writes them
//! as a new version. A document that is no filling-in of the template (its graph was edited) is customized: Settings
//! then only names it, with Reset to default. `crate::builtin_settings` serves one of these as a Settings route.

use super::*;
use crate::workflow::templates::unfill;
use pdt_core::workflow::template::{TemplateParam, fill, param_values};
use serde_json::{Map, Value};

/// See [`InputsBuiltin::finish`].
pub type Finish = fn(WorkflowDoc, &Map<String, Value>) -> Result<WorkflowDoc, String>;
/// See [`InputsBuiltin::read`].
pub type Read = fn(&WorkflowDoc, &mut Map<String, Value>);

pub struct InputsBuiltin {
    /// Its built-in id ([`BUILTINS`]).
    pub id: &'static str,
    pub params: fn() -> Vec<TemplateParam>,
    /// The document with `{{params.*}}` in it.
    pub template: fn() -> WorkflowDoc,
    /// What filling in leaves to code (values derived from the inputs, checks across them): given the filled-in
    /// document and the inputs, checked. None: every input is a plain `{{params.*}}` reference.
    pub finish: Option<Finish>,
    /// Reads back what `finish` wrote into a document (into the inputs read so far).
    pub read: Option<Read>,
}

impl InputsBuiltin {
    /// The inputs' defaults.
    pub fn defaults(&self) -> Map<String, Value> {
        param_values(&(self.params)(), &Map::new()).expect("a built-in's defaults are valid")
    }

    /// The workflow filled in with `values` (checked; missing ones take their defaults).
    pub fn filled(&self, values: &Map<String, Value>) -> Result<WorkflowDoc, String> {
        let v = param_values(&(self.params)(), values)?;
        let d = fill(&(self.template)(), &v)?;
        match self.finish {
            Some(finish) => finish(d, &v),
            None => Ok(d),
        }
    }

    /// The built-in as PDT ships it.
    pub fn shipped(&self) -> WorkflowDoc {
        self.filled(&Map::new()).expect("a built-in fills in with its defaults")
    }

    /// The inputs a document was filled in with: each one read back from it, else its default.
    pub fn values_of(&self, doc: &WorkflowDoc) -> Map<String, Value> {
        let mut out = self.defaults();
        for (k, v) in unfill(&(self.template)(), doc) {
            if out.contains_key(&k) {
                out.insert(k, v);
            }
        }
        if let Some(read) = self.read {
            read(doc, &mut out);
        }
        out
    }

    /// Whether a document is the workflow filled in with some inputs (its name and description aside): else the user
    /// changed it beyond its inputs.
    pub fn customized(&self, doc: &WorkflowDoc) -> bool {
        match self.filled(&self.values_of(doc)) {
            Ok(mut d) => {
                (d.name, d.description) = (doc.name.clone(), doc.description.clone());
                d != *doc
            }
            Err(_) => true,
        }
    }

    /// A newer version of a document: the template filled in with the document's inputs.
    pub fn refill(&self, doc: &WorkflowDoc) -> Result<WorkflowDoc, String> {
        self.filled(&self.values_of(doc))
    }
}
