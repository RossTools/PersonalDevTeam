//! `attempt.merge`: the Merge command as an automated merger. The project's policy decides whether
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

/// A first merge refused for something the agent can fix (PDT-139): PR notices puts the Coder on the attempt's later
/// conflicts and CI failures (not set to merge, so CI passing alone turns nothing on: PDT-124), and a merge again
/// after a fix finishes the ticket as this one asked.
async fn refused_fixable(cx: &StepCx<'_>, attempt: &str, req: &MergeRequest) {
    let store = &cx.st.store;
    if let Some(finish) = req.finish_on_merge
        && let Err(e) = store.set_attempt_finish(attempt, finish, req.delete_branch.unwrap_or(false)).await
    {
        tracing::warn!(attempt, error = %e, "could not record how a refused merge finishes");
    }
    if let Err(e) = store.set_attempt_merge_refused(attempt).await {
        tracing::warn!(attempt, error = %e, "could not record a refused merge");
    }
}

impl NodeKind for Merge {
    fn spec(&self) -> KindSpec {
        spec::<MergeConfig, Value>(
            KIND,
            1,
            Category::Action,
            "Merge the attempt",
            "Merges the attempt as Merge… does: sets its pull request to merge (`queued`: auto-merge, its merge \
             queue, or, where the repository has no auto-merge, PDT merges it once CI passes), or merges it locally or \
             at once (`merged`). The project's policy may not let a workflow merge, or may need a person's approval \
             first; that, and anything that stops the merge, leaves by `refused`. With `again` (after a fix), a pull \
             request already set to merge counts as `queued`, and the first merge's finishing choice stays. A refusal \
             says `fixable` when the attempt's agent can clear it with a commit (a conflict, uncommitted changes, \
             failing CI). With the project's `requireGreenCi` (on by default) a pull request merges only once CI on \
             its head passed: `code` `ci_failed` (fixable) or `ci_pending` otherwise; a repository with no CI \
             merges as before, noted on the ticket. Merging `again` before CI passed sets the pull request to merge, \
             so CI passing merges it.",
            &[("merged", "Merged"), ("queued", "Set to merge"), ("refused", "Refused")],
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
                    if r.fixable && !c.again {
                        refused_fixable(cx, &c.attempt, &req).await;
                    }
                    // Merging again after a fix, before the fix's CI passed (PDT-151): set to merge, so CI passing
                    // merges it (PR notices) and a failure gets fixed again.
                    if c.again
                        && r.code.starts_with("ci_")
                        && let Err(e) = cx.st.store.set_attempt_merge_requested(&c.attempt).await
                    {
                        tracing::warn!(attempt = %c.attempt, error = %e, "could not record a merge waiting for CI");
                    }
                    let mut out = json!({"code": r.code, "reason": r.reason, "fixable": r.fixable});
                    // The head CI was judged at (PDT-151): a wait for CI matches its events, earlier ones too.
                    if r.code.starts_with("ci_")
                        && let Some(pr) = crate::github::watched_pr(cx.st, &c.attempt)
                    {
                        out["headSha"] = json!(pr.head_sha);
                    }
                    Ok(Outcome::next("refused", out))
                }
                Err(e) => Err(e.message().to_string()),
            }
        })
    }

    fn default_timeout_ms(&self) -> u64 {
        2 * 60 * 1000
    }
}
