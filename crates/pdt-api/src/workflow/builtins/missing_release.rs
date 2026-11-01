//! The Report a missing scheduled release template.

use super::*;

/// Report a missing scheduled release (PDT-294, after PDT-288's watch for PDT's nightlies): a release workflow on
/// a schedule (GitHub's are best-effort, and some days never start) should have published a release whose tag matches
/// a pattern by an hour. Each day then it looks; a missing one goes to the Inbox with Start it (the workflow, with the
/// inputs given), and the item closes by itself once the release is published.
pub(crate) fn missing_release_template() -> pdt_core::workflow::template::Template {
    use crate::workflow::templates::{builtin, params};
    builtin(
        MISSING_RELEASE,
        Category::Releases,
        params(json!([
            {"id": "repo", "title": "Repository", "type": "string", "required": true,
             "description": "owner/name on GitHub."},
            {"id": "workflow", "title": "Release workflow", "type": "string", "required": true,
             "description": "The scheduled release workflow's file (publish.yml): its runs are linked, and Start it \
                             runs it."},
            {"id": "branch", "title": "Branch", "type": "string", "default": "main",
             "description": "The branch the release is built from (and Start it runs on)."},
            {"id": "tagPattern", "title": "Tag pattern", "type": "string", "required": true,
             "description": "The scheduled release's tag: * is any text, ? one character and {date} the day \
                             (YYYYMMDD or YYYY-MM-DD), e.g. v*-beta.{date}* or snapshot-{date}. Without {date}, a \
                             matching release published that day counts."},
            {"id": "hour", "title": "Expected by (hour, UTC)", "type": "number", "default": 10,
             "description": "When the day's release should be out (0 to 23): it looks then."},
            {"id": "onlyIfChanged", "title": "Only if the branch changed", "type": "boolean", "default": true,
             "description": "Say nothing on a day the branch has nothing the newest matching release lacks."},
            {"id": "inputs", "title": "Start it inputs", "type": "text", "default": "{}",
             "description": "The workflow's inputs for Start it, as JSON: {\"channel\": \"beta\"}."},
            {"id": "title", "title": "Inbox title", "type": "string", "default": "A scheduled release is missing"},
        ])),
        doc(json!({
            "name": "Report a missing scheduled release",
            "description": "Every day at the hour you give, looks for the day's release from a scheduled release \
                            workflow (by its tag pattern). When none is published (and the branch changed since \
                            the last one), the Inbox says so, with Start it, which runs the workflow; the item \
                            closes by itself once the release is published.",
            "nodes": [
                {"id": "daily", "kind": "schedule", "version": 1, "position": {"x": 0, "y": 0}, "title": "Every day",
                 "config": {"cron": "0 {{params.hour}} * * *", "timezone": "UTC", "catchUp": true}},
                {"id": "watch", "kind": "github.nightly", "version": 2, "position": {"x": 260, "y": 0},
                 "title": "The day's release", "retry": {"max": 5, "backoffMs": 60_000},
                 "config": {"repo": "{{params.repo}}", "workflow": "{{params.workflow}}", "branch": "{{params.branch}}",
                            "tagPattern": "{{params.tagPattern}}", "day": "{{trigger.date}}",
                            "dueBy": "{{params.hour}}:00", "onlyIfChanged": "{{params.onlyIfChanged}}",
                            "title": "{{params.title}}"}},
                {"id": "start", "kind": "github.workflow.run", "version": 1, "position": {"x": 520, "y": -160},
                 "title": "Run the release workflow", "config": {"repo": "{{params.repo}}",
                    "workflow": "{{params.workflow}}", "ref": "{{params.branch}}", "inputs": "{{params.inputs}}"}},
                {"id": "started", "kind": "notify", "version": 1, "position": {"x": 780, "y": -160},
                 "title": "Tell me it started", "config": {
                    "title": "The release workflow is starting",
                    "body": "{{params.workflow}} runs on {{params.branch}}.",
                    "link": "{{steps.watch.runsUrl}}"}},
                {"id": "start_failed", "kind": "inbox.notice", "version": 1, "position": {"x": 780, "y": -320},
                 "title": "Inbox: it would not start", "config": {
                    "key": "release-start-failed-{{steps.watch.day}}",
                    "title": "The release workflow didn't start",
                    "text": "PDT could not run {{params.workflow}}: {{steps.start.error}}\n\nStart it on GitHub \
                             instead: [the release workflow]({{steps.watch.runsUrl}}), Run workflow on \
                             {{params.branch}}."}},
                {"id": "done", "kind": "end", "version": 1, "position": {"x": 1040, "y": -160}, "title": "Started"},
                {"id": "fine", "kind": "end", "version": 1, "position": {"x": 520, "y": 160}, "title": "Fine",
                 "config": {"quiet": true}},
            ],
            "edges": [
                {"from": "daily", "to": "watch"},
                {"from": "watch", "port": "start", "to": "start"},
                {"from": "start", "to": "started"},
                {"from": "start", "port": "error", "to": "start_failed"},
                {"from": "started", "to": "done"},
                {"from": "start_failed", "port": "added", "to": "done"},
                {"from": "start_failed", "port": "skipped", "to": "done"},
                {"from": "watch", "port": "published", "to": "fine"},
                {"from": "watch", "port": "early", "to": "fine"},
                {"from": "watch", "port": "unchanged", "to": "fine"},
                {"from": "watch", "port": "ignore", "to": "fine"},
                {"from": "watch", "port": "already", "to": "fine"},
                {"from": "watch", "port": "expired", "to": "fine"},
            ],
        })),
    )
}
