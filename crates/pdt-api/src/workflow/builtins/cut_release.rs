//! The Cut a release template.

use super::*;

/// How often "Cut a release" looks, and for how long: the release PR (opened within a minute or two), its merge (it
/// may merge itself once its checks and a merge queue pass: hours; a failed check waits for the user's fix, up to two
/// weeks) and the published release (bundles may build for an hour or more; allowed four).
const PR_POLL_MS: u64 = 30_000;

const PR_POLLS: u32 = 40;

const MERGE_POLL_MS: u64 = 5 * 60_000;

const MERGE_POLLS: u32 = 14 * 24 * 12;

const RELEASE_POLL_MS: u64 = 2 * 60_000;

const RELEASE_POLLS: u32 = 4 * 30;

/// Cut a release (PDT-194, PDT-214, PDT-294; a template since PDT-301): made for a project from its release workflow
/// (`workflow`, run on `ref`), it asks Run now for the kind of release and follows what the workflow makes:
/// - a stable release runs the workflow with `stableInputs` (`{{trigger.bump}}` or `{{trigger.version}}` in them).
///   With `releaseBranch` (the branch the workflow opens its release PR from), an open one stops a new release, and the
///   run says when the PR is open, puts a failed check in the Inbox and waits for the merge; without it, there is no PR
///   stage. `tagPattern` is its tag (none: the latest release);
/// - the other kind (named `preName`, e.g. Nightly) runs it with `preInputs` and has no PR; `preTagPattern` is its tag.
///
/// It says when the release is published. PDT merges and tags nothing: the workflow and the forge do.
pub(crate) fn cut_release_template() -> pdt_core::workflow::template::Template {
    use crate::workflow::templates::{builtin, params};
    // A GitHub read that fails (the network, a rate limit) is tried again before the run fails.
    let retry = json!({"max": 5, "backoffMs": 60_000});
    let wait = |id: &str, x: i32, y: i32, title: &str, ms: u64, visits: u32| {
        json!({"id": id, "kind": "wait", "version": 1, "position": {"x": x, "y": y}, "title": title,
               "maxVisits": visits, "config": {"ms": ms}})
    };
    let run_workflow = |id: &str, x: i32, y: i32, inputs: &str| {
        json!({"id": id, "kind": "github.workflow.run", "version": 1, "position": {"x": x, "y": y},
               "title": "Run the release workflow", "config": {"workflow": "{{params.workflow}}",
               "ref": "{{params.ref}}", "inputs": inputs}})
    };
    let release = |id: &str, x: i32, y: i32, title: &str, tag: &str, extra: serde_json::Value| {
        let mut config = json!({"tagPattern": tag});
        config.as_object_mut().unwrap().extend(extra.as_object().cloned().unwrap_or_default());
        json!({"id": id, "kind": "github.release", "version": 1, "position": {"x": x, "y": y}, "title": title,
               "retry": retry, "config": config})
    };
    // While the release isn't out, the release workflow's run (PDT-488): a failed or cancelled one goes to the Inbox,
    // once per attempt, and it keeps waiting (re-running the failed jobs reuses the tag). `p` prefixes the node ids,
    // `wait` is the wait it goes back to, `what` names the release, `find` picks the run.
    let follow = |p: &str, wait: &str, x: i32, y: i32, what: &str, find: serde_json::Value| {
        let id = |s: &str| format!("{p}{s}");
        let out = |field: &str| format!("{{{{steps.{p}release_run.{field}}}}}");
        let key = |kind: &str| format!("release-run-{}-{}-{kind}", out("id"), out("attempt"));
        let mut config = json!({"workflow": "{{params.workflow}}"});
        config.as_object_mut().unwrap().extend(find.as_object().cloned().unwrap_or_default());
        let keeps_waiting = "Cut a release keeps waiting, and says when the release is published.";
        let nodes = vec![
            json!({"id": id("release_run"), "kind": "github.workflow.latest", "version": 1,
             "position": {"x": x, "y": y + 200}, "title": "Release run failed?", "retry": retry,
             "maxVisits": RELEASE_POLLS, "config": config}),
            json!({"id": id("run_failed"), "kind": "inbox.notice", "version": 1,
             "position": {"x": x + 260, "y": y + 200}, "title": "Inbox: the release run failed",
             "maxVisits": RELEASE_POLLS, "config": {
                "key": key("failed"),
                "title": format!("{what} failed to publish"),
                "text": format!("[The release run]({}) failed, so nothing is published yet.\n\n{}\n\nRe-run the \
                                 failed jobs on GitHub (that reuses the tag), or fix it and cut again. {keeps_waiting}",
                                out("url"), out("failedJobs"))}}),
            json!({"id": id("notify_run_failed"), "kind": "notify", "version": 1,
             "position": {"x": x + 520, "y": y + 200}, "title": "Tell me it failed", "maxVisits": RELEASE_POLLS,
             "config": {
                "title": format!("{what} failed to publish"),
                "body": "Re-run the failed jobs on GitHub (that reuses the tag), or fix it and cut again.",
                "level": "warning", "link": out("url")}}),
            json!({"id": id("run_cancelled"), "kind": "inbox.notice", "version": 1,
             "position": {"x": x + 260, "y": y + 400}, "title": "Inbox: the release run was cancelled",
             "maxVisits": RELEASE_POLLS, "config": {
                "key": key("cancelled"),
                "title": format!("{what} was cancelled"),
                "text": format!("[The release run]({}) was cancelled before it published the release. Re-run it on \
                                 GitHub, or cut again. {keeps_waiting}", out("url"))}}),
        ];
        let edges = vec![
            json!({"from": id("release_run"), "port": "passed", "to": wait}),
            json!({"from": id("release_run"), "port": "running", "to": wait}),
            json!({"from": id("release_run"), "port": "none", "to": wait}),
            json!({"from": id("release_run"), "port": "failed", "to": id("run_failed")}),
            json!({"from": id("release_run"), "port": "cancelled", "to": id("run_cancelled")}),
            json!({"from": id("run_failed"), "port": "added", "to": id("notify_run_failed")}),
            json!({"from": id("run_failed"), "port": "skipped", "to": wait}),
            json!({"from": id("notify_run_failed"), "to": wait}),
            json!({"from": id("run_cancelled"), "port": "added", "to": wait}),
            json!({"from": id("run_cancelled"), "port": "skipped", "to": wait}),
        ];
        (nodes, edges)
    };
    // A release without a PR (a stable one without releaseBranch, or the other kind): the workflow publishes it.
    let direct = |p: &str, y: i32, inputs: &str, tag: &str, what: &str| {
        let id = |s: &str| format!("{p}{s}");
        let mut published = release(
            &id("published"),
            1040,
            y,
            "Release published?",
            tag,
            json!({"newerThan": format!("{{{{steps.{p}before.tag}}}}")}),
        );
        published["maxVisits"] = json!(RELEASE_POLLS);
        let out = |field: &str| format!("{{{{steps.{p}published.{field}}}}}");
        // The run it started: the newest the dispatch started on its branch.
        let (run_nodes, run_edges) = follow(
            p,
            &id("wait"),
            1040,
            y,
            what,
            json!({"branch": "{{params.ref}}", "event": "workflow_dispatch",
                   "since": format!("{{{{steps.{p}dispatch.dispatchedAt}}}}")}),
        );
        let mut nodes = vec![
            release(&id("before"), 260, y, "Newest release before", tag, json!({})),
            run_workflow(&id("dispatch"), 520, y, inputs),
            wait(&id("wait"), 780, y, "Wait for the release", RELEASE_POLL_MS, RELEASE_POLLS),
            published,
            json!({"id": id("notify"), "kind": "notify", "version": 1, "position": {"x": 1300, "y": y},
             "title": "Tell me it is out", "config": {
                "title": format!("Release {} is published", out("tag")),
                "body": format!("{} is out.", out("name")),
                "link": out("url")}}),
            json!({"id": id("tell"), "kind": "inbox.notice", "version": 1, "position": {"x": 1560, "y": y},
             "title": "Inbox: published", "config": {
                "key": format!("released-{}", out("tag")),
                "title": format!("Release {} is published", out("tag")),
                "text": format!("[{}]({}) is published.", out("name"), out("url"))}}),
            json!({"id": id("released"), "kind": "end", "version": 1, "position": {"x": 1820, "y": y},
             "title": "Released", "config": {"output": {"tag": out("tag"), "url": out("url")}}}),
        ];
        let edges = vec![
            json!({"from": id("before"), "port": "found", "to": id("dispatch")}),
            json!({"from": id("before"), "port": "none", "to": id("dispatch")}),
            json!({"from": id("dispatch"), "to": id("wait")}),
            json!({"from": id("wait"), "port": "done", "to": id("published")}),
            json!({"from": id("published"), "port": "found", "to": id("notify")}),
            json!({"from": id("published"), "port": "none", "to": id("release_run")}),
            json!({"from": id("notify"), "to": id("tell")}),
            json!({"from": id("tell"), "port": "added", "to": id("released")}),
            json!({"from": id("tell"), "port": "skipped", "to": id("released")}),
        ];
        nodes.extend(run_nodes);
        let mut edges = edges;
        edges.extend(run_edges);
        (nodes, edges)
    };
    let (direct_nodes, direct_edges) =
        direct("direct_", -900, "{{params.stableInputs}}", "{{params.tagPattern}}", "The stable release");
    let (pre_nodes, pre_edges) =
        direct("pre_", -1600, "{{params.preInputs}}", "{{params.preTagPattern}}", "The {{params.preName}} release");
    // The release PR's merge starts the run: on its head branch, at its head commit.
    let (run_nodes, run_edges) = follow(
        "",
        "wait_release",
        2340,
        200,
        "{{steps.check_pr.title}}",
        json!({"branch": "{{steps.pr_branch.value}}", "event": "pull_request", "headSha": "{{steps.check_pr.headSha}}"}),
    );
    let mut published = release(
        "published",
        2340,
        0,
        "Release published?",
        "{{params.tagPattern}}",
        json!({"newerThan": "{{steps.before.tag}}"}),
    );
    published["maxVisits"] = json!(RELEASE_POLLS);
    let start = json!({"id": "start", "kind": "manual", "version": 1, "position": {"x": -520, "y": 0},
        "title": "Cut a release", "config": {
        "inputs": [
            {"id": "type", "label": "Type", "type": "choice", "required": true, "default": "{{params.defaultType}}",
             "choices": [{"value": "prerelease", "label": "{{params.preName}}"}, {"value": "stable", "label": "Stable"}]},
            {"id": "version", "label": "Exact version", "type": "text", "showIf": {"field": "type", "equals": "stable"},
             "help": "X.Y.Z. Leave it empty to bump the version instead."},
            {"id": "bump", "label": "Bump", "type": "choice", "default": "patch",
             "showIf": {"field": "version", "equals": ""},
             "choices": [{"value": "patch", "label": "Patch"}, {"value": "minor", "label": "Minor"},
                         {"value": "major", "label": "Major"}]},
        ],
        "confirm": {"text": "This publishes a stable release ({{trigger.bump}}{{trigger.version}}). Continue?",
                    "showIf": {"field": "type", "equals": "stable"}}}});
    let mut nodes = vec![
        start,
        json!({"id": "kind", "kind": "condition", "version": 1, "position": {"x": -260, "y": 0},
         "title": "Stable?", "config": {"conditions": [{"left": "{{trigger.type}}", "op": "eq", "right": "stable"}]}}),
        json!({"id": "has_pr", "kind": "condition", "version": 1, "position": {"x": 0, "y": 0},
         "title": "A release PR?", "config": {"conditions": [{"left": "{{params.releaseBranch}}", "op": "truthy"}]}}),
    ];
    nodes.extend(direct_nodes);
    nodes.extend(pre_nodes);
    nodes.extend(run_nodes);
    // A release PR: the workflow opens it, and it merges (by itself or by hand), which publishes the release. Its
    // branch comes through a step, so a project without one (these nodes never run) still validates.
    nodes.extend([
        json!({"id": "pr_branch", "kind": "state.set", "version": 1, "position": {"x": 130, "y": 100},
         "title": "The release PR branch", "config": {"key": "releaseBranch", "value": "{{params.releaseBranch}}"}}),
        release("before", 260, 0, "Latest release before", "{{params.tagPattern}}", json!({})),
        // A workflow that opens one release PR at a time refuses while one is open, and that PR isn't the release
        // asked for: say so instead.
        json!({"id": "open_already", "kind": "github.change_request", "version": 1, "position": {"x": 390, "y": 200},
         "title": "A release PR still open?", "retry": retry,
         "config": {"branch": "{{steps.pr_branch.value}}", "dryRunPort": "none"}}),
        json!({"id": "still_open", "kind": "inbox.notice", "version": 1, "position": {"x": 650, "y": 200},
         "title": "Inbox: one is still open", "config": {
            "key": "release-pr-open-{{steps.open_already.number}}",
            "title": "Release PR #{{steps.open_already.number}} is still open",
            "text": "No new release was started: [{{steps.open_already.title}}]({{steps.open_already.url}}) is \
                     still open on {{steps.pr_branch.value}}. Merge it (that release ships) or close it, then \
                     run Cut a release again."}}),
        json!({"id": "not_started", "kind": "end", "version": 1, "position": {"x": 910, "y": 200},
         "title": "Not started"}),
        run_workflow("dispatch", 520, 0, "{{params.stableInputs}}"),
        wait("wait_pr", 780, 0, "Wait for the release PR", PR_POLL_MS, PR_POLLS),
        json!({"id": "find_pr", "kind": "github.change_request", "version": 1, "position": {"x": 1040, "y": 0},
         "title": "Release PR opened?", "retry": retry, "maxVisits": PR_POLLS,
         "config": {"branch": "{{steps.pr_branch.value}}"}}),
        json!({"id": "tell_pr", "kind": "notify", "version": 1, "position": {"x": 1300, "y": 0},
         "title": "Tell me it is open", "config": {
            "title": "Release PR #{{steps.find_pr.number}} is open",
            "body": "{{steps.find_pr.title}}: the release is published once it merges.",
            "link": "{{steps.find_pr.url}}"}}),
        wait("wait_merge", 1560, 0, "Wait for the merge", MERGE_POLL_MS, MERGE_POLLS),
        json!({"id": "check_pr", "kind": "github.change_request", "version": 1, "position": {"x": 1820, "y": 0},
         "title": "Release PR merged?", "retry": retry, "maxVisits": MERGE_POLLS,
         "config": {"branch": "{{steps.pr_branch.value}}", "number": "{{steps.find_pr.number}}", "ci": true,
                    "dryRunPort": "merged"}}),
        // Once per head commit and kind of failure: a fix pushed that fails again is news again.
        json!({"id": "tell_failed", "kind": "inbox.notice", "version": 1, "position": {"x": 1820, "y": -200},
         "title": "Inbox: a check failed", "maxVisits": MERGE_POLLS, "config": {
            "key": "release-pr-{{steps.check_pr.number}}-{{steps.check_pr.failure}}-{{steps.check_pr.headSha}}",
            "title": "Release PR #{{steps.check_pr.number}} failed its checks",
            "text": "[{{steps.check_pr.title}}]({{steps.check_pr.url}}) didn't merge: [a check failed]\
                     ({{steps.check_pr.ciUrl}}).\n\n{{steps.check_pr.failedJobs}}\n\nIt stays open, and nothing \
                     is released. Push a fix to {{steps.pr_branch.value}} (after a merge queue removed it, set it \
                     to merge again), or close it, which ends this release."}}),
        json!({"id": "notify_failed", "kind": "notify", "version": 1, "position": {"x": 1560, "y": -200},
         "title": "Tell me it failed", "maxVisits": MERGE_POLLS, "config": {
            "title": "Release PR #{{steps.check_pr.number}} failed its checks",
            "body": "It stays open and nothing is released: fix it or close it.",
            "level": "warning", "link": "{{steps.check_pr.ciUrl}}"}}),
        wait("wait_release", 2080, 0, "Wait for the release", RELEASE_POLL_MS, RELEASE_POLLS),
        published,
        json!({"id": "notify", "kind": "notify", "version": 1, "position": {"x": 2600, "y": 0},
         "title": "Tell me it is out", "config": {
            "title": "Release {{steps.published.tag}} is published",
            "body": "{{steps.published.name}} is out.",
            "link": "{{steps.published.url}}"}}),
        json!({"id": "tell_released", "kind": "inbox.notice", "version": 1, "position": {"x": 2860, "y": 0},
         "title": "Inbox: published", "config": {
            "key": "released-{{steps.published.tag}}",
            "title": "Release {{steps.published.tag}} is published",
            "text": "[{{steps.published.name}}]({{steps.published.url}}) is published."}}),
        json!({"id": "released", "kind": "end", "version": 1, "position": {"x": 3120, "y": 0}, "title": "Released",
         "config": {"output": {"tag": "{{steps.published.tag}}", "url": "{{steps.published.url}}"}}}),
        json!({"id": "dropped", "kind": "notify", "version": 1, "position": {"x": 2080, "y": 200},
         "title": "Tell me it was closed", "config": {
            "title": "Release PR #{{steps.find_pr.number}} was closed without merging",
            "body": "Nothing was released. Run Cut a release again for a new one.",
            "level": "warning", "link": "{{steps.find_pr.url}}"}}),
        json!({"id": "not_released", "kind": "end", "version": 1, "position": {"x": 2340, "y": 200},
         "title": "Not released"}),
    ]);
    let mut edges = vec![
        json!({"from": "start", "to": "kind"}),
        json!({"from": "kind", "port": "true", "to": "has_pr"}),
        json!({"from": "kind", "port": "false", "to": "pre_before"}),
        json!({"from": "has_pr", "port": "true", "to": "pr_branch"}),
        json!({"from": "pr_branch", "to": "before"}),
        json!({"from": "has_pr", "port": "false", "to": "direct_before"}),
    ];
    edges.extend(direct_edges);
    edges.extend(pre_edges);
    edges.extend(run_edges);
    edges.extend([
        json!({"from": "before", "port": "found", "to": "open_already"}),
        json!({"from": "before", "port": "none", "to": "open_already"}),
        json!({"from": "open_already", "port": "open", "to": "still_open"}),
        json!({"from": "open_already", "port": "none", "to": "dispatch"}),
        json!({"from": "open_already", "port": "merged", "to": "dispatch"}),
        json!({"from": "open_already", "port": "closed", "to": "dispatch"}),
        json!({"from": "still_open", "port": "added", "to": "not_started"}),
        json!({"from": "still_open", "port": "skipped", "to": "not_started"}),
        json!({"from": "dispatch", "to": "wait_pr"}),
        json!({"from": "wait_pr", "port": "done", "to": "find_pr"}),
        json!({"from": "find_pr", "port": "open", "to": "tell_pr"}),
        // Until the new one is open, the branch's newest PR may be the last release's (merged).
        json!({"from": "find_pr", "port": "none", "to": "wait_pr"}),
        json!({"from": "find_pr", "port": "merged", "to": "wait_pr"}),
        json!({"from": "find_pr", "port": "closed", "to": "wait_pr"}),
        json!({"from": "tell_pr", "to": "wait_merge"}),
        json!({"from": "wait_merge", "port": "done", "to": "check_pr"}),
        json!({"from": "check_pr", "port": "open", "to": "wait_merge"}),
        json!({"from": "check_pr", "port": "failed", "to": "tell_failed"}),
        json!({"from": "tell_failed", "port": "added", "to": "notify_failed"}),
        json!({"from": "tell_failed", "port": "skipped", "to": "wait_merge"}),
        json!({"from": "notify_failed", "to": "wait_merge"}),
        json!({"from": "check_pr", "port": "merged", "to": "wait_release"}),
        json!({"from": "check_pr", "port": "closed", "to": "dropped"}),
        json!({"from": "check_pr", "port": "none", "to": "dropped"}),
        json!({"from": "dropped", "to": "not_released"}),
        json!({"from": "wait_release", "port": "done", "to": "published"}),
        json!({"from": "published", "port": "found", "to": "notify"}),
        json!({"from": "published", "port": "none", "to": "release_run"}),
        json!({"from": "notify", "to": "tell_released"}),
        json!({"from": "tell_released", "port": "added", "to": "released"}),
        json!({"from": "tell_released", "port": "skipped", "to": "released"}),
    ]);
    builtin(
        CUT_RELEASE,
        Category::Releases,
        params(json!([
            {"id": "workflow", "title": "Release workflow", "type": "string", "required": true,
             "description": "The release workflow's file (publish.yml), started by hand (workflow_dispatch)."},
            {"id": "ref", "title": "Branch it runs on", "type": "string", "default": "main"},
            {"id": "stableInputs", "title": "Stable release inputs", "type": "text",
             "default": "{\"bump\": \"{{trigger.bump}}\", \"version\": \"{{trigger.version}}\"}",
             "description": "The workflow's inputs for a stable release, as JSON. {{trigger.bump}} is the bump \
                             asked for (patch, minor or major), {{trigger.version}} the exact version; empty ones are \
                             left out."},
            {"id": "releaseBranch", "title": "Release PR branch", "type": "string", "default": "",
             "description": "When the workflow opens a release PR for a stable release, the branch it opens it from: \
                             you are told when it is open, a failed check goes to the Inbox, and an open one stops a \
                             new release. Empty: no release PR."},
            {"id": "tagPattern", "title": "Stable tag pattern", "type": "string", "default": "",
             "description": "The stable release's tag (* is any text), e.g. v*. Empty: the latest release."},
            {"id": "preName", "title": "Other release's name", "type": "string", "default": "Pre-release",
             "description": "What Run now calls the other kind of release, e.g. Beta."},
            {"id": "preInputs", "title": "Other release's inputs", "type": "text", "default": "{}",
             "description": "The workflow's inputs for it, as JSON: {\"stage\": \"beta\"}. It has no release PR."},
            {"id": "preTagPattern", "title": "Other release's tag pattern", "type": "string", "default": "",
             "description": "Its tag (* is any text), e.g. v*-beta.*; pre-releases count. Empty: the newest release."},
            {"id": "defaultType", "title": "Run now offers first", "type": "choice", "default": "prerelease",
             "choices": ["prerelease", "stable"],
             "description": "The kind Run now's form starts on. A stable release always asks you to confirm."},
        ])),
        crate::workflow::templates::doc(json!({
            "name": "Cut a release",
            "description": "Runs a project's release workflow and tells you when the release is published. Run now \
                            asks which kind: a stable release (an exact version, or a patch, minor or major bump; \
                            you confirm it) or the other kind the workflow makes (a pre-release). With a release PR \
                            branch, an open release PR stops a new stable release, you are told when the PR is open, \
                            and a failed check goes to the Inbox while it waits for the merge. Run it (the PM does, \
                            when you ask for a release) with input {\"type\": \"stable\", \"bump\": \"minor\"}, \
                            {\"type\": \"stable\", \"version\": \"2.0.0\"} or {\"type\": \"prerelease\"}.",
            "nodes": nodes,
            "edges": edges,
            "settings": {"maxSteps": 2 * PR_POLLS + 3 * MERGE_POLLS + 5 * RELEASE_POLLS + 20,
                         "maxRuntimeMs": 16_i64 * 24 * 60 * 60 * 1000},
        })),
    )
}
