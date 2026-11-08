//! `github.workflow.run` (PDT-194): start a CI workflow by hand, e.g. the repository's Release Action, through the
//! run's project's GitHub Actions (else GitHub's account). `repo` defaults to the run's project's GitHub repository.

use super::forge_call::ci_call;
use super::repo_of::{check_repo, repo_for_step, templated};
use crate::workflow::catalog::{
    BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, nonblank, parse, repo_missing, spec,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct GithubWorkflowRunConfig {
    /// The repository (`owner/name`); the run's project's when left out.
    #[serde(default)]
    pub repo: Option<String>,
    /// The workflow's file name (`publish.yml`) or id. It must take `workflow_dispatch`.
    pub workflow: String,
    /// The branch or tag it runs on (default `main`).
    #[serde(default, rename = "ref")]
    pub git_ref: Option<String>,
    /// Its `workflow_dispatch` inputs (an object, or text holding one as JSON); empty and null values are left out.
    #[serde(default)]
    #[ts(type = "Record<string, unknown> | null")]
    pub inputs: Option<Value>,
}

/// Inputs given as text: the JSON object it holds (a template's text param, PDT-294); blank is none.
fn inputs_text(s: &str) -> Result<Value, String> {
    match s.trim() {
        "" => Ok(Value::Null),
        t => match serde_json::from_str::<Value>(t) {
            Ok(v @ Value::Object(_)) => Ok(v),
            _ => Err("inputs is not an object (or JSON text holding one)".into()),
        },
    }
}

/// The inputs as GitHub takes them: text, without the empty ones.
fn inputs_of(v: &Option<Value>) -> Result<BTreeMap<String, String>, String> {
    let Some(v) = v else { return Ok(BTreeMap::new()) };
    let parsed;
    let v = match v {
        Value::String(s) => {
            parsed = inputs_text(s)?;
            &parsed
        }
        v => v,
    };
    let Some(m) = v.as_object() else {
        return match v {
            Value::Null => Ok(BTreeMap::new()),
            _ => Err("inputs is not an object".into()),
        };
    };
    Ok(m.iter()
        .filter_map(|(k, v)| {
            let s = match v {
                Value::Null => return None,
                Value::String(s) => s.trim().to_string(),
                other => other.to_string(),
            };
            (!s.is_empty()).then(|| (k.clone(), s))
        })
        .collect())
}

pub struct GithubWorkflowRun;

impl NodeKind for GithubWorkflowRun {
    fn needs_project(&self, config: &Value) -> bool {
        repo_missing(config)
    }

    fn spec(&self) -> KindSpec {
        spec::<GithubWorkflowRunConfig, Value>(
            "github.workflow.run",
            1,
            Category::Action,
            "Run a GitHub Actions workflow",
            "Starts a GitHub Actions workflow by hand (`workflow_dispatch`) on a branch (default `main`) with inputs, \
             as PDT's GitHub login. The repository is the run's project's unless `repo` names one. GitHub doesn't \
             say which run it started: read what it makes (a pull request, a release) with the other GitHub nodes, \
             or the run with `github.workflow.latest` and `since` set to its `dispatchedAt`.",
            &[("out", "Then")],
            false,
        )
    }

    /// Started twice, the workflow runs twice.
    fn rerun_safe(&self, _input: &Value) -> bool {
        false
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: GithubWorkflowRunConfig = parse(config)?;
        if c.workflow.trim().is_empty() {
            return Err("workflow is empty: name its file (publish.yml)".into());
        }
        check_repo(&c.repo)?;
        match &c.inputs {
            None | Some(Value::Null | Value::Object(_)) => Ok(()),
            Some(Value::String(s)) if templated(s) => Ok(()),
            Some(Value::String(s)) => inputs_text(s).map(|_| ()),
            Some(_) => Err("inputs is not an object".into()),
        }
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: GithubWorkflowRunConfig = parse(&input)?;
            let of = repo_for_step(cx, &c.repo).await?;
            let repo = of.repo.clone();
            let workflow = c.workflow.trim().to_string();
            if workflow.is_empty() {
                return Err("workflow is empty: name its file (publish.yml)".into());
            }
            let git_ref = nonblank(&c.git_ref).unwrap_or("main").to_string();
            let inputs = inputs_of(&c.inputs)?;
            // When it was started: `github.workflow.latest`'s `since` finds the run it started (PDT-488).
            let at = jiff::Timestamp::now().round(jiff::Unit::Second).unwrap_or_else(|_| jiff::Timestamp::now());
            let out = json!({"repo": repo, "workflow": workflow, "ref": git_ref, "inputs": inputs, "dispatchedAt": at.to_string()});
            if cx.dry_run {
                return Ok(Outcome::would(
                    "out",
                    format!("run {workflow} on {repo} at {git_ref} with {}", json!(inputs)),
                ));
            }
            let actions = of.actions(&format!("run {workflow} with"))?;
            let (r, w, g) = (repo, workflow, git_ref);
            ci_call(&actions, move |a| a.trigger(&r, &g, &w, &inputs)).await?;
            Ok(Outcome::next("out", out))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configs_are_checked() {
        let run = |v: Value| GithubWorkflowRun.check(&v);
        assert!(run(json!({"workflow": "release.yml", "inputs": {"bump": "{{trigger.bump}}"}})).is_ok());
        assert!(run(json!({"workflow": " "})).is_err());
        assert!(run(json!({"workflow": "r.yml", "repo": "nope"})).unwrap_err().contains("owner/name"));
        assert!(run(json!({"workflow": "r.yml", "inputs": [1]})).is_err());
        assert!(run(json!({"workflow": "r.yml", "inputs": "{\"channel\": \"beta\"}"})).is_ok());
        assert!(run(json!({"workflow": "r.yml", "inputs": "  "})).is_ok());
        assert!(run(json!({"workflow": "r.yml", "inputs": "channel=beta"})).is_err());
    }

    #[test]
    fn inputs_are_text_without_empty_ones() {
        let i = inputs_of(&Some(json!({"bump": "minor", "version": "", "dry": true, "n": null}))).unwrap();
        assert_eq!(i, BTreeMap::from([("bump".into(), "minor".into()), ("dry".into(), "true".into())]));
        assert!(inputs_of(&None).unwrap().is_empty());
        assert!(inputs_of(&Some(json!("x"))).is_err());
        let text = inputs_of(&Some(json!(r#"{"channel": "beta", "n": 2}"#))).unwrap();
        assert_eq!(text, BTreeMap::from([("channel".into(), "beta".into()), ("n".into(), "2".into())]));
        assert!(inputs_of(&Some(json!(""))).unwrap().is_empty());
    }
}
