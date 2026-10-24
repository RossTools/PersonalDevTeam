//! `attempt.merge`: the Merge command as an automated merger (Phase 7c spec §5). The project's policy decides whether
//! a workflow may merge; what stops a merge (a conflict, a draft PR, no PR in a project that merges through them) and
//! a policy refusal leave by `refused`, saying whether the attempt's agent can fix it (`fixable`, PDT-124).

use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::policy::Merger;
use pdt_core::review::{MergeOutcome, MergeRequest, MergeStyle};
use pdt_core::workflow::tpl::Tpl;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

pub const KIND: &str = "attempt.merge";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct MergeConfig {
    /// The attempt (`{{trigger.attemptId}}`).
    pub attempt: String,
    /// A local merge's style (a pull request merges the way GitHub's settings say).
    #[serde(default)]
    pub style: MergeStyle,
    /// A pull request: move the ticket to Done and clean up once it merges.
    #[serde(default = "yes")]
    #[ts(type = "boolean | string")]
    pub finish_on_merge: Tpl<bool>,
    /// With `finishOnMerge`: delete the branch too.
    #[serde(default)]
    #[ts(type = "boolean | string")]
    pub delete_branch: Tpl<bool>,
    /// Merging again after a failure (PDT-104): `finishOnMerge` and `deleteBranch` stay as the first merge asked.
    #[serde(default)]
    pub again: bool,
    /// The ticket's pipeline stage while it merges (the Board's column).
    #[serde(default)]
    #[ts(optional)]
    pub pipeline_stage: Option<pdt_core::ticket::PipelineStage>,
}

fn yes() -> Tpl<bool> {
    Tpl::Value(true)
}

pub struct Merge;

/// The attempt's open pull request as GitHub has it now (a push just changed it, so the last look may be stale).
async fn fresh_pr(cx: &StepCx<'_>, attempt: &str) -> Option<pdt_core::pr::PullRequestStatus> {
    let info = cx.st.store.get_attempt(attempt).await.ok().flatten()?;
    crate::github::refresh_attempt(cx.st, &info).await;
    crate::github::watched_pr(cx.st, attempt).filter(|p| p.state == pdt_core::pr::PrState::Open)
}

impl NodeKind for Merge {
    fn spec(&self) -> KindSpec {
        spec::<MergeConfig, Value>(
            KIND,
            1,
            Category::Action,
            "Merge the attempt",
            "Merges the attempt as Merge… does: turns on auto-merge for its pull request (`queued`), or merges it \
             locally (`merged`). The project's policy may not let a workflow merge, or may need a person's approval \
             first; that, and anything that stops the merge, leaves by `refused`. With `again` (after a fix), a pull \
             request already set to merge counts as `queued`, and the first merge's finishing choice stays. A refusal \
             says `fixable` when the attempt's agent can clear it with a commit (a conflict, uncommitted changes).",
            &[("merged", "Merged"), ("queued", "Auto-merge on"), ("refused", "Refused")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parse::<MergeConfig>(config).map(|_| ())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: MergeConfig = parse(&input)?;
            if cx.dry_run {
                return Ok(Outcome::next("queued", json!({"would": format!("merge attempt {}", c.attempt)})));
            }
            // Cut short after it merged (PDT-131): the attempt says so. A pull request already set to merge is `queued`
            // below as for any automated merge.
            if cx.rerun
                && let Ok(Some(info)) = cx.st.store.get_attempt(&c.attempt).await
                && info.attempt.state == pdt_core::ticket::AttemptState::Merged
            {
                return Ok(Outcome::next("merged", json!({"outcome": MergeOutcome::Merged, "already": true})));
            }
            if c.again
                && let Some(pr) = fresh_pr(cx, &c.attempt).await
                && (pr.auto_merge || pr.in_merge_queue)
            {
                return Ok(Outcome::next("queued", json!({"outcome": MergeOutcome::Queued, "already": true})));
            }
            crate::workflow::nodes::set_stage(cx, c.pipeline_stage).await;
            let req = MergeRequest {
                style: Some(c.style),
                finish_on_merge: (!c.again).then(|| c.finish_on_merge.or(true)),
                delete_branch: (!c.again).then(|| c.delete_branch.or(false)),
                by: None,
            };
            match crate::review::merge_attempt(cx.st, &c.attempt, &req, Merger::Automated).await {
                Ok(Ok(r)) => {
                    let port = if r.outcome == MergeOutcome::Merged { "merged" } else { "queued" };
                    Ok(Outcome::next(port, json!({"outcome": r.outcome, "commit": r.commit})))
                }
                Ok(Err(r)) => {
                    Ok(Outcome::next("refused", json!({"code": r.code, "reason": r.reason, "fixable": r.fixable})))
                }
                Err(e) => Err(e.message().to_string()),
            }
        })
    }

    fn default_timeout_ms(&self) -> u64 {
        2 * 60 * 1000
    }
}
