//! Keeping a pull request PDT didn't open moving once its base is fixed (PDT-641): `github.pr.update` brings it up to
//! date (Dependabot's with `@dependabot rebase`, others with the forge's update-branch), once per base fix; and
//! `github.pr.approval` records the user's approval with the pull request's own diff, so a later head whose own diff is
//! the same (only the base changed) keeps it. Both keep what they remember in the workflow's state, under PDT's own
//! keys, so it holds across the pull request's runs.

use super::forge_call::forge_call;
use super::repo_of::{RepoOf, check_repo, repo_for_step, templated};
use crate::routes::now_ms;
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, nonblank, parse, spec};
use crate::workflow::nodes::schedule::RESERVED_STATE;
use pdt_core::base_ci::{DiffFile, diff_id};
use pdt_core::outside_pr::DEPENDABOT;
use pdt_core::pr::PrState;
use pdt_forge::ChangeFile;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

/// What Dependabot is asked in a comment to rebase its pull request.
pub const DEPENDABOT_REBASE: &str = "@dependabot rebase";

/// What "approval kept" says on the run.
pub const APPROVAL_KEPT: &str = "Approval kept: only the base changed";

fn number(v: &Value) -> Result<u32, String> {
    let text = match v {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.trim().trim_start_matches('#').to_string(),
        _ => String::new(),
    };
    text.parse().map_err(|_| format!("{text:?} is not a pull request number"))
}

fn check_number(v: &Value) -> Result<(), String> {
    match v {
        Value::String(s) if templated(s) => Ok(()),
        n => number(n).map(|_| ()),
    }
}

/// The workflow-state key for what `what` remembers of a pull request.
fn key(what: &str, repo: &str, number: u32) -> String {
    format!("{RESERVED_STATE}{what}.{}#{number}", repo.to_lowercase())
}

fn own_diff(files: &[ChangeFile]) -> Option<String> {
    let files: Vec<DiffFile<'_>> = files
        .iter()
        .map(|f| DiffFile {
            path: &f.path,
            previous_path: f.previous_path.as_deref(),
            status: &f.status,
            patch: f.patch.as_deref(),
        })
        .collect();
    diff_id(&files)
}

// ---- github.pr.update ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct PrUpdateConfig {
    /// The repository (`owner/name`, `{{trigger.repo}}`); the run's project's when left out.
    #[serde(default)]
    pub repo: Option<String>,
    /// The pull request's number (`{{trigger.number}}`).
    #[ts(type = "number | string")]
    pub number: Value,
    /// Only while its head is this commit (`{{trigger.headSha}}`): `moved` otherwise.
    #[serde(default)]
    pub head_sha: Option<String>,
    /// Who opened it (`{{trigger.author}}`): Dependabot's is asked to rebase.
    #[serde(default)]
    pub author: Option<String>,
    /// What it is updated for (the base's fixed commit, `{{steps.base_now.sha}}`): once per key, never again.
    #[serde(default)]
    pub key: Option<String>,
}

pub struct PrUpdate;

impl NodeKind for PrUpdate {
    fn needs_project(&self, config: &Value) -> bool {
        crate::workflow::catalog::repo_missing(config)
    }

    fn spec(&self) -> KindSpec {
        spec::<PrUpdateConfig, Value>(
            "github.pr.update",
            1,
            Category::Action,
            "Bring a pull request up to date",
            "Updates a pull request with its base branch: Dependabot's gets a `@dependabot rebase` comment, any other \
             the forge's update-branch (it merges the base in, where the repository allows it). Once per `key` (the \
             base's fix): asked before, it leaves by `already` and does nothing. `updated` once asked; `moved` when \
             its head isn't `headSha`; `gone` when it isn't open; `refused` (with `reason`) when the forge won't.",
            &[
                ("updated", "Asked to update"),
                ("already", "Asked before"),
                ("moved", "Pushed since"),
                ("gone", "Not open"),
                ("refused", "Refused"),
            ],
            false,
        )
    }

    fn outward(&self, _config: &Value) -> bool {
        true
    }

    /// Asked once per key: a second run finds it asked.
    fn rerun_safe(&self, _input: &Value) -> bool {
        true
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: PrUpdateConfig = parse(config)?;
        check_repo(&c.repo)?;
        check_number(&c.number)
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: PrUpdateConfig = parse(&input)?;
            let n = number(&c.number)?;
            let dependabot = nonblank(&c.author).is_some_and(|a| a.eq_ignore_ascii_case(DEPENDABOT));
            if cx.dry_run {
                let how = if dependabot { "comment @dependabot rebase on" } else { "update the branch of" };
                return Ok(Outcome::would("updated", format!("{how} #{n}")));
            }
            let RepoOf { repo, forge, .. } = repo_for_step(cx, &c.repo).await?;
            let r = repo.clone();
            let pr = forge_call(&forge, move |f| f.change_request_number(&r, n)).await?;
            let Some(pr) = pr.filter(|p| p.state == PrState::Open) else {
                return Ok(Outcome::next("gone", json!({"quiet": true})));
            };
            let head = nonblank(&c.head_sha).map(String::from);
            if head.as_deref().is_some_and(|h| h != pr.head_sha) {
                return Ok(Outcome::next("moved", json!({"headSha": pr.head_sha, "quiet": true})));
            }
            let state_key = key("update", &repo, n);
            let fix = nonblank(&c.key).unwrap_or(&pr.base_branch).to_string();
            let store = &cx.st.store;
            let state = store.workflow_state(&cx.workflow.id).await.map_err(crate::workflow::err)?;
            if state.get(&state_key).and_then(Value::as_str) == Some(fix.as_str()) {
                return Ok(Outcome::next("already", json!({"key": fix, "quiet": true})));
            }
            let r = repo.clone();
            let asked = forge_call(&forge, move |f| {
                if dependabot {
                    f.comment_change_request(&r, n, DEPENDABOT_REBASE).map(|url| ("rebase", Some(url)))
                } else {
                    f.update_change_request_branch(&r, n, head.as_deref()).map(|()| ("update-branch", None))
                }
            })
            .await;
            match asked {
                Ok((how, url)) => {
                    store
                        .set_workflow_state(&cx.workflow.id, &state_key, &json!(fix), false, now_ms())
                        .await
                        .map_err(crate::workflow::err)?;
                    Ok(Outcome::next("updated", json!({"how": how, "commentUrl": url, "key": fix})))
                }
                Err(reason) => Ok(Outcome::next("refused", json!({"reason": reason}))),
            }
        })
    }
}

// ---- github.pr.approval ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct PrApprovalConfig {
    /// The repository (`owner/name`, `{{trigger.repo}}`); the run's project's when left out.
    #[serde(default)]
    pub repo: Option<String>,
    /// The pull request's number (`{{trigger.number}}`).
    #[ts(type = "number | string")]
    pub number: Value,
    /// Its head (`{{trigger.headSha}}`): the one approved, or the one to check.
    pub head_sha: String,
    /// `record` (the user approved this head), `forget` (they decided otherwise since) or `check` (does an approval
    /// hold for it).
    pub action: String,
}

pub struct PrApproval;

impl NodeKind for PrApproval {
    fn needs_project(&self, config: &Value) -> bool {
        crate::workflow::catalog::repo_missing(config)
    }

    fn spec(&self) -> KindSpec {
        spec::<PrApprovalConfig, Value>(
            "github.pr.approval",
            1,
            Category::Action,
            "The user's approval of a pull request",
            "`record`: remembers that the user approved the pull request at `headSha`, with an id of its own diff \
             (against its merge base, as git's patch-id: line numbers left out); a pull request whose head is no \
             longer `headSha` (pushed since) records nothing and leaves by `moved`. `forget`: the user decided \
             otherwise since (Request changes, Close): the approval is gone. `check`: whether an approval holds for \
             `headSha`: `kept` when it is the head approved, or its own diff is the one approved (only the base \
             changed, as after a rebase; output `note`), `changed` when its diff changed, `none` when there is none. \
             A diff with a file the forge shows no patch for (binary, too large) never carries an approval.",
            &[
                ("out", "Recorded or forgotten"),
                ("moved", "Pushed since"),
                ("kept", "Approval holds"),
                ("changed", "Diff changed"),
                ("none", "Not approved"),
            ],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: PrApprovalConfig = parse(config)?;
        check_repo(&c.repo)?;
        if !["record", "check", "forget"].contains(&c.action.trim()) {
            return Err(format!("action {:?} is not record, check or forget", c.action));
        }
        if c.head_sha.trim().is_empty() {
            return Err("headSha is empty ({{trigger.headSha}})".into());
        }
        check_number(&c.number)
    }

    fn rerun_safe(&self, _input: &Value) -> bool {
        true
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: PrApprovalConfig = parse(&input)?;
            let n = number(&c.number)?;
            let head = c.head_sha.trim().to_string();
            let action = c.action.trim();
            if cx.dry_run {
                return Ok(match action {
                    "record" => Outcome::would("out", format!("remember the approval of #{n} at {head}")),
                    "forget" => Outcome::would("out", format!("forget any approval of #{n}")),
                    _ => {
                        Outcome::next("none", json!({"would": format!("check for an approval of #{n}"), "quiet": true}))
                    }
                });
            }
            let store = &cx.st.store;
            let RepoOf { repo, forge, .. } = repo_for_step(cx, &c.repo).await?;
            let state_key = key("approval", &repo, n);
            let set = async |value: &Value| {
                store
                    .set_workflow_state(&cx.workflow.id, &state_key, value, false, now_ms())
                    .await
                    .map_err(crate::workflow::err)
            };
            if action == "forget" {
                set(&Value::Null).await?;
                return Ok(Outcome::next("out", json!({"forgotten": true, "quiet": true})));
            }
            let state = store.workflow_state(&cx.workflow.id).await.map_err(crate::workflow::err)?;
            let approved = state.get(&state_key).cloned().filter(|a| !a.is_null());
            let record = action == "record";
            if !record && approved.is_none() {
                return Ok(Outcome::next("none", json!({"quiet": true})));
            }
            if !record && approved.as_ref().and_then(|a| a["headSha"].as_str()) == Some(head.as_str()) {
                return Ok(Outcome::next("kept", json!({"note": "Approved at this head", "headSha": head})));
            }
            // The files are the head's at the time they are read: only while it is still the head approved (read
            // before and after them) are they the approved diff.
            let (r, h) = (repo.clone(), head.clone());
            let read = forge_call(&forge, move |f| {
                let head_now = |f: &dyn pdt_forge::Forge| {
                    f.change_request_number(&r, n).map(|p| p.map(|p| p.head_sha).unwrap_or_default())
                };
                let before = head_now(f)?;
                // A list cut off (more files than the forge lists) is no diff: no approval is kept over it (PDT-696).
                let files = f.change_request_files_whole(&r, n)?;
                let after = head_now(f)?;
                Ok((before == h && after == h).then(|| files.and_then(|f| own_diff(&f))).ok_or(after))
            })
            .await?;
            if record {
                let diff = match read {
                    Ok(diff) => diff,
                    Err(now) => return Ok(Outcome::next("moved", json!({"headSha": now, "quiet": true}))),
                };
                set(&json!({"headSha": head, "diff": diff, "at": now_ms()})).await?;
                return Ok(Outcome::next("out", json!({"headSha": head, "diff": diff, "quiet": true})));
            }
            let was = approved.as_ref().and_then(|a| a["diff"].as_str()).map(String::from);
            let approved_head = approved.as_ref().and_then(|a| a["headSha"].as_str()).unwrap_or_default().to_string();
            match (was, read.ok().flatten()) {
                (Some(a), Some(b)) if a == b => Ok(Outcome::next(
                    "kept",
                    json!({"note": APPROVAL_KEPT, "approvedHead": approved_head, "headSha": head}),
                )),
                _ => Ok(Outcome::next("changed", json!({"approvedHead": approved_head, "headSha": head}))),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configs_are_checked() {
        let pr = json!({"repo": "{{trigger.repo}}", "number": "{{trigger.number}}"});
        assert!(PrUpdate.check(&pr).is_ok());
        assert!(PrUpdate.check(&json!({"repo": "nope", "number": 3})).is_err());
        let a = |action: &str| json!({"number": 3, "headSha": "{{trigger.headSha}}", "action": action});
        assert!(PrApproval.check(&a("record")).is_ok());
        assert!(PrApproval.check(&a("check")).is_ok());
        assert!(PrApproval.check(&a("forget")).is_ok());
        assert!(PrApproval.check(&a("drop")).unwrap_err().contains("record, check or forget"));
        assert_eq!(key("approval", "O/R", 7), "_pdt.approval.o/r#7");
    }
}
