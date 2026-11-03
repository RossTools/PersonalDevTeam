//! Conflicts with the base, fixed once at merge time (PDT-328). `attempt.defer_conflict`: whether a conflict (or a base
//! that moved) waits for merge time instead of going to the attempt's agent now. `attempt.update_base`: PDT merges the
//! base into the branch itself, regenerates generated files by the project's `[[merge.regenerate]]` rules, pushes once,
//! and hands only real conflicts to an agent, with the files and hunks.

use crate::routes::{AppState, now_ms};
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use crate::workflow::nodes::command;
use pdt_core::brand::PROJECT_CONFIG_FILE;
use pdt_core::command_trust;
use pdt_core::limits::OUTPUT_CAP;
use pdt_core::merge_update::{self, ConflictNote, MergeUpdateHow, MergeUpdateStatus};
use pdt_core::runconfig::RegenerateDef;
use pdt_core::ticket::{Actor, EventKind, PipelineStage};
use pdt_store::AttemptInfo;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;
use ts_rs::TS;

pub const DEFER_KIND: &str = "attempt.defer_conflict";
pub const UPDATE_KIND: &str = "attempt.update_base";

/// How long one regeneration command may run.
const REGENERATE_LIMIT: Duration = Duration::from_secs(30 * 60);
/// The most merged-since subjects the agent's message lists.
const MAX_MERGED: usize = 30;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct DeferConfig {
    /// The attempt (`{{trigger.attemptId}}`).
    pub attempt: String,
    /// The notice's kind (`{{trigger.notice.kind}}`): a deferred `conflict` is recorded on the ticket and shown.
    #[serde(default)]
    #[ts(optional)]
    pub notice: Option<String>,
}

pub struct DeferConflict;

impl NodeKind for DeferConflict {
    fn spec(&self) -> KindSpec {
        spec::<DeferConfig, Value>(
            DEFER_KIND,
            1,
            Category::Condition,
            "Resolve at merge time?",
            "`deferred` when the attempt's conflict waits for merge time: its project resolves conflicts at merge \
             time (the default), its ticket is in a Pipeline stage before the merge (code, review, QA or accept) and \
             it isn't set to merge. A deferred conflict is recorded on the ticket's timeline and shown on it. \
             Otherwise `now`.",
            &[("deferred", "Resolved at merge time"), ("now", "Resolve it now")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parse::<DeferConfig>(config).map(|_| ())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: DeferConfig = parse(&input)?;
            let st = cx.st;
            let Some(info) = st.store.get_attempt(&c.attempt).await.map_err(crate::workflow::err)? else {
                return Ok(Outcome::next("now", json!({"deferred": false})));
            };
            let policy = st.store.get_project_settings(&info.project_id).await.map_err(crate::workflow::err)?.policy;
            let stage =
                st.store.get_ticket_summary(&info.ticket_id).await.ok().flatten().and_then(|t| t.pipeline_stage);
            let pr = crate::github::watched_pr(st, &info.attempt.id);
            let set_to_merge = info.merge_requested
                || info.merge_refused
                || pr.as_ref().is_some_and(|p| crate::github::merge_requested(&info, p));
            let deferred = merge_update::defers(policy.resolve_conflicts, stage, set_to_merge);
            if !deferred {
                return Ok(Outcome::next("now", json!({"deferred": false})));
            }
            if cx.dry_run {
                return Ok(Outcome::next("deferred", json!({"deferred": true})));
            }
            if c.notice.as_deref() == Some("conflict") {
                crate::github::set_merge_update(st, &info.attempt.id, Some(MergeUpdateStatus::Deferred));
                let pr = pr.map(|p| p.number);
                let data = event_data(&info, MergeUpdateHow::Deferred, json!({"pr": pr}));
                add_event(st, &info, None, data).await;
            }
            Ok(Outcome::next("deferred", json!({"deferred": true})))
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct UpdateConfig {
    /// The attempt (`{{trigger.attemptId}}`).
    pub attempt: String,
    /// The ticket's pipeline stage while it updates (the Board's column).
    #[serde(default)]
    #[ts(optional)]
    pub pipeline_stage: Option<PipelineStage>,
}

pub struct UpdateBase;

impl NodeKind for UpdateBase {
    fn spec(&self) -> KindSpec {
        spec::<UpdateConfig, Value>(
            UPDATE_KIND,
            1,
            Category::Action,
            "Update with the base",
            "Brings the attempt's branch up to date with its base in its worktree, with no agent: fetches the base \
             (`origin/<base>` when the project has an origin), merges it in and pushes the branch once. `current`: \
             it had the base already. `updated`: the base merged in cleanly, or only generated files conflicted and \
             the project's `[[merge.regenerate]]` rules regenerated them (the base's side, then the rule's command, \
             committed). `conflict`: other files conflict; the merge is left in progress for an agent and `message` \
             lists the files, the hunks and what merged into the base since. `skipped`: it couldn't (no worktree, \
             uncommitted changes, the base or the push unreachable), leaving the branch as it was. Each result goes \
             on the ticket's timeline.",
            &[("current", "Up to date"), ("updated", "Updated"), ("conflict", "Conflicts"), ("skipped", "Skipped")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parse::<UpdateConfig>(config).map(|_| ())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: UpdateConfig = parse(&input)?;
            if cx.dry_run {
                return Ok(Outcome::next("current", json!({"would": format!("update attempt {}", c.attempt)})));
            }
            let st = cx.st;
            let Some(info) = st.store.get_attempt(&c.attempt).await.map_err(crate::workflow::err)? else {
                let d = skipped("the attempt is gone");
                return Ok(Outcome::next(d.port, d.output));
            };
            crate::workflow::nodes::set_stage(cx, c.pipeline_stage).await;
            crate::github::set_merge_update(st, &info.attempt.id, Some(MergeUpdateStatus::Updating));
            let out = update(st, &info).await;
            let status = (out.port == "conflict").then_some(MergeUpdateStatus::Resolving);
            crate::github::set_merge_update(st, &info.attempt.id, status);
            record(st, &info, &out).await;
            if out.port == "updated" {
                // The pushed head, for the merge that follows.
                crate::github::refresh_attempt(st, &info).await;
            }
            Ok(Outcome::next(out.port, out.output))
        })
    }

    fn default_timeout_ms(&self) -> u64 {
        // Regeneration commands build (cargo, pnpm).
        REGENERATE_LIMIT.as_millis() as u64 + 10 * 60 * 1000
    }
}

/// How an update ended: the port it leaves by, and its output.
struct Done {
    port: &'static str,
    output: Value,
}

fn done(port: &'static str, output: Value) -> Done {
    Done { port, output }
}

fn skipped(reason: &str) -> Done {
    done("skipped", json!({"reason": reason}))
}

/// The regeneration rules of the base branch's committed configuration, never the worktree's (an agent can write that
/// one; PDT-405): those whose command the user approved, and those that wait on their approval.
async fn regenerate_rules(st: &AppState, info: &AttemptInfo) -> (Vec<RegenerateDef>, Vec<RegenerateDef>) {
    let (repo, base) = (PathBuf::from(&info.repo_path), info.attempt.base_branch.clone());
    let file = tokio::task::spawn_blocking(move || pdt_git::branch_file(&repo, &base, PROJECT_CONFIG_FILE))
        .await
        .map_err(crate::workflow::err)
        .and_then(|r| r.map_err(crate::workflow::err));
    let config = match file {
        Ok(f) => crate::runconfig::effective_for_file(st, &info.project_id, f.as_deref())
            .await
            .map_err(|e| e.message().to_string()),
        Err(e) => Err(e),
    };
    let rules = match config {
        Ok(c) => c.regenerate,
        Err(e) => {
            tracing::warn!(ticket = %info.ticket_key, error = %e, "no regeneration rules: the configuration can't be read");
            return (vec![], vec![]);
        }
    };
    match crate::command_trust::trusted(st, &info.project_id, &info.repo_path).await {
        Ok(trusted) => rules
            .into_iter()
            .partition(|r| trusted.contains(&command_trust::of_regenerate(&r.run, &r.paths).fingerprint)),
        Err(e) => {
            tracing::warn!(ticket = %info.ticket_key, error = %e.message(), "no regeneration rules: approvals can't be read");
            (vec![], vec![])
        }
    }
}

/// What the update did, as the node's outcome.
async fn update(st: &AppState, info: &AttemptInfo) -> Done {
    let Some(worktree) = info.worktree_path.as_deref().map(PathBuf::from).filter(|p| p.is_dir()) else {
        return skipped("the attempt has no worktree");
    };
    let pr = crate::github::watched_pr(st, &info.attempt.id).filter(|p| p.state == pdt_core::pr::PrState::Open);
    let remote_pr = pr.as_ref().is_some_and(|p| !p.local);
    let (repo, base, branch) =
        (PathBuf::from(&info.repo_path), info.attempt.base_branch.clone(), info.attempt.branch.clone());
    let wt = worktree.clone();
    let prepared = tokio::task::spawn_blocking(move || prepare(&repo, &wt, &base, &branch, remote_pr))
        .await
        .map_err(crate::workflow::err)
        .and_then(|r| r);
    let p = match prepared {
        Ok(p) => p,
        Err(why) => return skipped(&why),
    };
    match p.merged {
        Merged::Current => done("current", json!({"how": "current", "base": p.base_ref})),
        Merged::Clean => finish(&worktree, &p, "clean", &[]).await,
        Merged::Conflicts(ref files) => {
            let (rules, waiting) = regenerate_rules(st, info).await;
            // Rules for these files whose command the user hasn't approved: their files are the agent's, and the Inbox
            // says the rule waits on the user.
            let unapproved = merge_update::plan(files, &waiting).rules;
            if !unapproved.is_empty() {
                let commands: Vec<_> = unapproved
                    .iter()
                    .map(|&i| command_trust::of_regenerate(&waiting[i].run, &waiting[i].paths))
                    .collect();
                let what = format!("Regenerating files when {} is brought up to date", info.ticket_key);
                crate::command_trust::notify(st, &info.project_id, &commands, &what).await;
            }
            let plan = merge_update::plan(files, &rules);
            let wt = worktree.clone();
            let generated = plan.generated.clone();
            let taken = tokio::task::spawn_blocking(move || pdt_git::take_theirs(&wt, &generated)).await;
            if let Err(e) = taken.map_err(crate::workflow::err).and_then(|r| r.map_err(crate::workflow::err)) {
                return conflict(&worktree, &p, files, &[], Some(("git checkout --theirs", &e))).await;
            }
            let commands: Vec<String> = plan.rules.iter().map(|&i| rules[i].run.clone()).collect();
            if !plan.real.is_empty() {
                return conflict(&worktree, &p, &plan.real, &commands, None).await;
            }
            for &i in &plan.rules {
                let rule = &rules[i];
                if let Err(why) = run_command(&worktree, &rule.run, st.platform.login_path.as_deref()).await {
                    return conflict(&worktree, &p, &[], &commands, Some((&rule.run, &why))).await;
                }
                let (wt, globs) = (worktree.clone(), rule.paths.clone());
                let staged = tokio::task::spawn_blocking(move || pdt_git::stage_globs(&wt, &globs)).await;
                if let Err(e) = staged.map_err(crate::workflow::err).and_then(|r| r.map_err(crate::workflow::err)) {
                    return conflict(&worktree, &p, &[], &commands, Some((&rule.run, &e))).await;
                }
            }
            let wt = worktree.clone();
            let committed = tokio::task::spawn_blocking(move || {
                let left = pdt_git::unmerged_files(&wt)?;
                if !left.is_empty() {
                    return Ok(Err(left));
                }
                pdt_git::commit_merge(&wt).map(Ok)
            })
            .await;
            match committed {
                Ok(Ok(Ok(_))) => finish(&worktree, &p, "regenerated", &plan.generated).await,
                Ok(Ok(Err(left))) => conflict(&worktree, &p, &left, &commands, None).await,
                Ok(Err(e)) => conflict(&worktree, &p, &[], &commands, Some(("git commit", &e.to_string()))).await,
                Err(e) => conflict(&worktree, &p, &[], &commands, Some(("git commit", &e.to_string()))).await,
            }
        }
    }
}

/// How the base merged in.
enum Merged {
    Current,
    Clean,
    Conflicts(Vec<String>),
}

/// The merge of the base, done ([`prepare`]).
struct Prepared {
    merged: Merged,
    /// What was merged (`origin/main`).
    base_ref: String,
    /// The branch's head before.
    before: String,
    /// Where the branch is pushed (`origin`), if anywhere.
    push: Option<String>,
    branch: String,
}

/// Fetches the base and merges it into the worktree's branch. Err: why it was left alone.
fn prepare(repo: &Path, worktree: &Path, base: &str, branch: &str, remote_pr: bool) -> Result<Prepared, String> {
    if pdt_git::merge_in_progress(worktree) {
        // An agent's resolution not finished: its conflicts are still the agent's.
        let files = pdt_git::unmerged_files(worktree).map_err(crate::workflow::err)?;
        if files.is_empty() {
            return Err("a merge is in progress in the worktree".into());
        }
        let before = pdt_git::rev_parse(worktree, "HEAD").unwrap_or_default();
        let base_ref = pdt_git::rev_parse(worktree, "MERGE_HEAD").unwrap_or_else(|| base.to_string());
        return Ok(Prepared { merged: Merged::Conflicts(files), base_ref, before, push: None, branch: branch.into() });
    }
    if pdt_git::is_dirty(worktree).map_err(crate::workflow::err)? {
        return Err("the worktree has uncommitted changes".into());
    }
    let origin = pdt_git::remote_url(repo, "origin").is_some();
    let base_ref = if origin {
        pdt_git::fetch(repo, "origin", base, pdt_git::BACKGROUND_REMOTE_TIMEOUT)
            .map_err(|e| format!("couldn't fetch {base}: {}", crate::workflow::err(e)))?;
        // A pull request merges into the remote's base; a local merge into the newer of the two.
        if remote_pr { format!("origin/{base}") } else { pdt_git::start_point(repo, base) }
    } else {
        base.to_string()
    };
    let before = pdt_git::rev_parse(worktree, "HEAD").ok_or("the branch has no commits")?;
    // Pushed once already (or with a pull request on the remote): the update goes there too.
    let pushed = pdt_git::rev_parse(repo, &format!("refs/remotes/origin/{branch}")).is_some();
    let push = (origin && (remote_pr || pushed)).then(|| "origin".to_string());
    let merged = match pdt_git::merge_base_in(worktree, &base_ref)
        .map_err(|e| format!("couldn't merge {base_ref}: {}", crate::workflow::err(e)))?
    {
        pdt_git::BaseMerge::Current => Merged::Current,
        pdt_git::BaseMerge::Merged { .. } => Merged::Clean,
        pdt_git::BaseMerge::Conflicts { files } => Merged::Conflicts(files),
    };
    Ok(Prepared { merged, base_ref, before, push, branch: branch.into() })
}

/// The merge committed: pushes it (once), or undoes it when the push fails.
async fn finish(worktree: &Path, p: &Prepared, how: &str, regenerated: &[String]) -> Done {
    let mut pushed = false;
    if let Some(remote) = p.push.clone() {
        let (wt, refspec, before) = (worktree.to_path_buf(), format!("HEAD:refs/heads/{}", p.branch), p.before.clone());
        let result = tokio::task::spawn_blocking(move || {
            match pdt_git::push(&wt, &remote, &[&refspec], pdt_git::BACKGROUND_REMOTE_TIMEOUT) {
                Ok(pdt_git::PushOutcome::Pushed) => Ok(()),
                Ok(pdt_git::PushOutcome::Rejected(why)) => Err(format!("the branch moved on the remote: {why}")),
                Err(e) => Err(crate::workflow::err(e)),
            }
            // Only the merge just made goes: the worktree was clean before it.
            .map_err(|why| match pdt_git::undo_merge(&wt, &before) {
                Ok(()) => why,
                Err(e) => format!(
                    "{why}; and the merge couldn't be undone, so the worktree still has it: {}",
                    crate::workflow::err(e)
                ),
            })
        })
        .await;
        match result.map_err(crate::workflow::err).and_then(|r| r) {
            Ok(()) => pushed = true,
            Err(why) => return skipped(&format!("couldn't push the update: {why}")),
        }
    }
    done("updated", json!({"how": how, "base": p.base_ref, "regenerated": regenerated, "pushed": pushed}))
}

/// Real conflicts (or a regeneration that failed): the merge stays in progress for an agent, with the message.
async fn conflict(
    worktree: &Path,
    p: &Prepared,
    files: &[String],
    commands: &[String],
    failed: Option<(&str, &str)>,
) -> Done {
    let (wt, list, before, base_ref) = (worktree.to_path_buf(), files.to_vec(), p.before.clone(), p.base_ref.clone());
    let (hunks, merged) = tokio::task::spawn_blocking(move || {
        let hunks = if list.is_empty() { String::new() } else { pdt_git::conflict_diff(&wt, &list) };
        (hunks, pdt_git::first_parent_subjects(&wt, &before, &base_ref, MAX_MERGED))
    })
    .await
    .unwrap_or_default();
    let message = merge_update::conflict_message(&ConflictNote {
        base: &p.base_ref,
        files,
        hunks: &hunks,
        merged: &merged,
        regenerate: commands,
        failed,
    });
    done("conflict", json!({"how": "conflict", "base": p.base_ref, "files": files, "message": message}))
}

/// Runs a regeneration command in the worktree (the user's login PATH); Err: the end of its output.
async fn run_command(dir: &Path, command: &str, path: Option<&str>) -> Result<(), String> {
    #[cfg(windows)]
    let mut cmd = {
        let mut c = tokio::process::Command::new("cmd");
        c.arg("/C").arg(command);
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = tokio::process::Command::new("sh");
        c.arg("-c").arg(command);
        c.process_group(0);
        c
    };
    cmd.current_dir(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if let Some(p) = path {
        cmd.env("PATH", p);
    }
    let mut child = cmd.spawn().map_err(|e| format!("couldn't run it: {e}"))?;
    let mut guard = command::TreeGuard(child.id());
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let stderr = child.stderr.take().ok_or("no stderr")?;
    // Only the end is shown: what is kept of its output is bounded, however much it writes.
    let run = async {
        tokio::join!(command::read_tail(stdout, OUTPUT_CAP), command::read_tail(stderr, OUTPUT_CAP), child.wait())
    };
    let (stdout, stderr, status) = match tokio::time::timeout(REGENERATE_LIMIT, run).await {
        Err(_) => return Err(format!("stopped after {} minutes", REGENERATE_LIMIT.as_secs() / 60)),
        Ok((out, err, status)) => (out, err, status.map_err(crate::workflow::err)?),
    };
    guard.0 = None;
    if status.success() {
        return Ok(());
    }
    let text = format!("{}{}", String::from_utf8_lossy(&stdout), String::from_utf8_lossy(&stderr));
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(30)..].join("\n");
    Err(format!("exit {}\n{tail}", status.code().map_or("by a signal".into(), |c| c.to_string())))
}

fn event_data(info: &AttemptInfo, how: MergeUpdateHow, more: Value) -> Value {
    let mut d = json!({"attemptId": info.attempt.id, "how": how});
    if let (Some(d), Value::Object(more)) = (d.as_object_mut(), more) {
        d.extend(more);
    }
    d
}

async fn add_event(st: &AppState, info: &AttemptInfo, body: Option<&str>, data: Value) {
    if let Err(e) = st
        .store
        .add_ticket_event(&info.ticket_id, EventKind::MergeUpdate, Actor::System, body, Some(data), now_ms())
        .await
    {
        tracing::warn!(ticket = %info.ticket_key, error = %e, "could not record a merge update");
    }
}

/// Puts the update on the ticket's timeline: clean, regenerated or sent to an agent; and once the branch is up to date
/// after an agent had its conflicts, that the agent resolved them.
async fn record(st: &AppState, info: &AttemptInfo, out: &Done) {
    let o = &out.output;
    let base = o["base"].clone();
    match out.port {
        "updated" | "current" => {
            let last = st.store.latest_event_data(&info.ticket_id, EventKind::MergeUpdate).await.ok().flatten();
            let had_conflicts = last.is_some_and(|d| {
                d["attemptId"] == info.attempt.id.as_str() && d["how"] == json!(MergeUpdateHow::Conflict)
            });
            if had_conflicts {
                add_event(st, info, None, event_data(info, MergeUpdateHow::Agent, json!({"base": base}))).await;
            }
            match o["how"].as_str() {
                Some("clean") => {
                    add_event(st, info, None, event_data(info, MergeUpdateHow::Clean, json!({"base": base}))).await
                }
                Some("regenerated") => {
                    let more = json!({"base": base, "regenerated": o["regenerated"]});
                    add_event(st, info, None, event_data(info, MergeUpdateHow::Regenerated, more)).await;
                }
                _ => {}
            }
        }
        "conflict" => {
            let more = json!({"base": base, "files": o["files"]});
            add_event(st, info, None, event_data(info, MergeUpdateHow::Conflict, more)).await;
        }
        _ => {
            tracing::info!(ticket = %info.ticket_key, reason = %o["reason"], "left the branch as it was at merge time");
        }
    }
}
