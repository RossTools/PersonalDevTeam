//! `attempt.agent_working`: whether an agent step is on the attempt now (PDT-104). Read when the step runs, not when
//! the event was logged: two events of one look at a PR (a conflict and the queue drop it caused) both say no agent
//! is on it, but the first run's fix step is by the time the second run gets here (runs advance one at a time).
//! `attempt.has_changes`: whether the attempt has anything to merge (PDT-354).

use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::workflow::tpl::Tpl;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;
use ts_rs::TS;

pub const KIND: &str = "attempt.agent_working";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct AgentWorkingConfig {
    /// The attempt (`{{trigger.attemptId}}`).
    pub attempt: String,
    /// Already known to be busy (the event's `{{trigger.agentWorking}}`): `yes` without looking.
    #[serde(default)]
    #[ts(type = "boolean | string | null")]
    pub known: Option<Tpl<bool>>,
}

pub struct AgentWorking;

impl NodeKind for AgentWorking {
    fn spec(&self) -> KindSpec {
        spec::<AgentWorkingConfig, Value>(
            KIND,
            1,
            Category::Condition,
            "An agent on the attempt?",
            "`yes` while an agent step works on the attempt, in this workflow or another (bound to a session, or \
             waiting for a slot), else `no`. Read as the step runs, so of two runs from one event burst only the first \
             finds it free.",
            &[("yes", "An agent is on it"), ("no", "No agent on it")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parse::<AgentWorkingConfig>(config).map(|_| ())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: AgentWorkingConfig = parse(&input)?;
            let busy = c.known.is_some_and(|k| k.or(false))
                || cx.st.store.attempt_agent_busy(&c.attempt, &cx.run.id).await.map_err(crate::workflow::err)?;
            Ok(Outcome::next(if busy { "yes" } else { "no" }, json!({"working": busy})))
        })
    }
}

pub const HAS_CHANGES_KIND: &str = "attempt.has_changes";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct HasChangesConfig {
    /// The attempt (`{{trigger.attemptId}}`).
    pub attempt: String,
}

pub struct HasChanges;

/// Whether the attempt has something to merge: a pull request, commits of its own (now, or ever seen), or
/// uncommitted changes in its worktree. Anything it can't read counts as changes, so nothing is thrown away.
async fn has_changes(cx: &StepCx<'_>, attempt: &str) -> Result<bool, String> {
    let info = cx.st.store.get_attempt(attempt).await.map_err(crate::workflow::err)?.ok_or("no such attempt")?;
    if info.own_commits || crate::github::watched_pr(cx.st, attempt).is_some() {
        return Ok(true);
    }
    let repo = PathBuf::from(&info.repo_path);
    let worktree = info.worktree_path.map(PathBuf::from);
    let (branch, base, base_commit) = (info.attempt.branch, info.attempt.base_branch, info.base_commit);
    tokio::task::spawn_blocking(move || {
        let dirty = worktree.filter(|w| w.is_dir()).is_some_and(|w| pdt_git::is_dirty(&w).unwrap_or(true));
        // Against where the branch started and both bases: Start branches from `origin/<base>` when it is ahead of
        // the local base (PDT-45), whose newer commits are not the branch's own.
        let origin = format!("origin/{base}");
        let own = |head: String| pdt_git::has_own_commits(&repo, &head, &base_commit, &[&base, &origin]);
        dirty || pdt_git::rev_parse(&repo, &format!("refs/heads/{branch}")).is_none_or(own)
    })
    .await
    .map_err(crate::workflow::err)
}

impl NodeKind for HasChanges {
    fn spec(&self) -> KindSpec {
        spec::<HasChangesConfig, Value>(
            HAS_CHANGES_KIND,
            1,
            Category::Condition,
            "Anything to merge?",
            "`yes` when the attempt has something to merge: a pull request, commits its base branch lacks, or \
             uncommitted changes in its worktree. `no` when its branch is the same as its base (a ticket that needed \
             no change), so there is nothing to merge. Read as the step runs.",
            &[("yes", "Has changes"), ("no", "No changes")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parse::<HasChangesConfig>(config).map(|_| ())
    }

    /// It runs git in the attempt's repository, which may be slow: other runs and the engine's tick don't wait for it.
    fn in_engine_lock(&self, _category: Category) -> bool {
        false
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: HasChangesConfig = parse(&input)?;
            if cx.dry_run {
                return Ok(Outcome::next("yes", json!({"would": format!("look for changes on {}", c.attempt)})));
            }
            let changes = has_changes(cx, &c.attempt).await?;
            Ok(Outcome::next(if changes { "yes" } else { "no" }, json!({"changes": changes})))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_git_reading_condition_runs_outside_the_engine_lock() {
        assert!(!HasChanges.in_engine_lock(Category::Condition), "git may be slow: other runs don't wait for it");
        assert!(AgentWorking.in_engine_lock(Category::Condition), "a condition reads the engine's state under it");
    }
}
