//! Buildkite actions (Phase 7e spec §6.4): `buildkite.retry` (a build's failed jobs, else the whole build) and
//! `buildkite.trigger` (a new build), through a `buildkite` connection (pdt-api `buildkite.rs`).

use crate::buildkite::BuildkiteClient;
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::buildkite::{parse_build_url, retryable_jobs};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BuildOutput {
    #[ts(type = "number")]
    pub number: u64,
    pub url: String,
}

fn output(b: &Value) -> BuildOutput {
    BuildOutput { number: b["number"].as_u64().unwrap_or(0), url: b["web_url"].as_str().unwrap_or_default().into() }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct RetryConfig {
    #[schemars(extend("x-connection-kind" = "buildkite"))]
    pub connection: String,
    /// The build: its URL (`{{trigger.url}}`), or its number with `pipeline`.
    #[ts(type = "string | number")]
    pub build: Value,
    #[serde(default)]
    pub pipeline: Option<String>,
}

impl RetryConfig {
    /// The pipeline and build number, or why not (None while a reference is unresolved).
    fn target(&self) -> Result<Option<(String, u64)>, String> {
        let pipeline = self.pipeline.as_deref().map(str::trim).filter(|p| !p.is_empty());
        match &self.build {
            Value::String(s) if s.contains("{{") => Ok(None),
            Value::String(s) if s.contains("://") => {
                parse_build_url(s).map(Some).ok_or_else(|| format!("{s:?} is not a Buildkite build URL"))
            }
            v => {
                let n = v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()));
                match (n, pipeline) {
                    (Some(n), Some(p)) if !p.contains("{{") => Ok(Some((p.to_string(), n))),
                    (Some(_), Some(_)) => Ok(None),
                    (Some(_), None) => Err("a build number needs its pipeline".into()),
                    (None, _) => Err("build is neither a build URL nor a number".into()),
                }
            }
        }
    }
}

pub struct Retry;

impl NodeKind for Retry {
    fn spec(&self) -> KindSpec {
        spec::<RetryConfig, BuildOutput>(
            "buildkite.retry",
            1,
            Category::Action,
            "Retry a Buildkite build",
            "Retries a Buildkite build's failed jobs, or rebuilds it when none can be retried.",
            &[("out", "Then")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: RetryConfig = parse(config)?;
        if c.connection.trim().is_empty() {
            return Err("connection is empty: name a buildkite connection".into());
        }
        c.target().map(|_| ())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: RetryConfig = parse(&input)?;
            let (pipeline, number) = c.target()?.ok_or("build is not resolved")?;
            if cx.dry_run {
                return Ok(Outcome::next("out", json!({"would": format!("retry {pipeline} build {number}")})));
            }
            let client = BuildkiteClient::named(cx.st, &c.connection).await?;
            let build = client.build(&pipeline, number).await?;
            let jobs = retryable_jobs(&build);
            if jobs.is_empty() {
                let b = client.rebuild(&pipeline, number).await?;
                let mut out = serde_json::to_value(output(&b)).unwrap_or_default();
                out["rebuilt"] = json!(true);
                return Ok(Outcome::Next { port: "out".into(), output: out });
            }
            for (id, _) in &jobs {
                client.retry_job(&pipeline, number, id).await?;
            }
            let mut out = serde_json::to_value(output(&build)).unwrap_or_default();
            out["retried"] = json!(jobs.iter().map(|(_, n)| n).collect::<Vec<_>>());
            Ok(Outcome::Next { port: "out".into(), output: out })
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct TriggerBuildConfig {
    #[schemars(extend("x-connection-kind" = "buildkite"))]
    pub connection: String,
    /// The pipeline's slug.
    pub pipeline: String,
    pub branch: String,
    /// Default `HEAD` (the branch's newest commit).
    #[serde(default = "head")]
    pub commit: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

fn head() -> String {
    "HEAD".into()
}

pub struct TriggerBuild;

impl NodeKind for TriggerBuild {
    fn spec(&self) -> KindSpec {
        spec::<TriggerBuildConfig, BuildOutput>(
            "buildkite.trigger",
            1,
            Category::Action,
            "Start a Buildkite build",
            "Starts a build of a Buildkite pipeline for a branch (and commit); its number and URL are \
             `{{steps.<id>.number}}` and `{{steps.<id>.url}}`.",
            &[("out", "Then")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: TriggerBuildConfig = parse(config)?;
        for (field, v) in [("connection", &c.connection), ("pipeline", &c.pipeline), ("branch", &c.branch)] {
            if v.trim().is_empty() {
                return Err(format!("{field} is empty"));
            }
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: TriggerBuildConfig = parse(&input)?;
            if cx.dry_run {
                let what = format!("build {} on {} at {}", c.pipeline, c.branch, c.commit);
                return Ok(Outcome::next("out", json!({"would": what})));
            }
            let client = BuildkiteClient::named(cx.st, &c.connection).await?;
            let message =
                if c.message.trim().is_empty() { format!("Started by {}", cx.workflow.name) } else { c.message };
            let body = json!({"commit": c.commit, "branch": c.branch, "message": message, "env": c.env});
            let b = client.create_build(&c.pipeline, &body).await?;
            Ok(Outcome::next("out", output(&b)))
        })
    }
}
