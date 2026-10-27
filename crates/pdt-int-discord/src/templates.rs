//! The workflow templates that post to Discord (Phase 7e spec §5).

use pdt_api::sdk::templates::{builtin, doc, params};
use pdt_core::workflow::template::Template;
use serde_json::{Value, json};

/// The param of a Discord webhook connection.
fn discord_param() -> Value {
    json!({"id": "discord", "title": "Discord channel", "type": "connection", "connectionKind": "discord_webhook",
        "required": true, "description": "Where it posts: a Discord webhook connection (Settings → Connections)."})
}

/// A schedule's `every`.
fn every_param(default: &str) -> Value {
    json!({"id": "every", "title": "How often", "type": "string", "default": default,
        "description": "An interval: 1m, 15m, 1h, 1d (from midnight UTC). For set times, change the schedule to a \
                        cron expression after creating it."})
}

/// The templates that post to Discord: the Hermes jobs that need no agent (Phase 7e spec §5).
pub(crate) fn all() -> Vec<Template> {
    let mut out = vec![
        builtin(
            "github-issues-discord",
            params(json!([
                {"id": "repo", "title": "Repository", "type": "string", "required": true,
                 "description": "owner/name on GitHub (read through PDT's GitHub login)."},
                discord_param(),
            ])),
            doc(json!({
                "name": "New GitHub issues to Discord",
                "description": "Posts each new issue of a GitHub repository to a Discord channel. The first look at \
                                the repository only records where it is.",
                "nodes": [
                    {"id": "opened", "kind": "github.issue.opened", "version": 1, "position": {"x": 0, "y": 0},
                     "config": {"repos": ["{{params.repo}}"]}},
                    {"id": "post", "kind": "discord.post", "version": 1, "position": {"x": 320, "y": 0},
                     "retry": {"max": 3, "backoffMs": 30000}, "config": {"connection": "{{params.discord}}",
                        "content": "New issue in {{trigger.repo}} #{{trigger.number}}: **{{trigger.title}}** by \
                                    {{trigger.author}}\n{{trigger.url}}"}},
                ],
                "edges": [{"from": "opened", "to": "post"}],
            })),
        ),
        builtin(
            "health-watchdog",
            params(json!([
                {"id": "name", "title": "Service name", "type": "string", "default": "The service"},
                {"id": "url", "title": "Health URL", "type": "string", "required": true,
                 "description": "Healthy when it answers with a status below 400."},
                every_param("1m"),
                discord_param(),
            ])),
            doc(json!({
                "name": "Health watchdog",
                "description": "Checks a URL on a schedule and posts to Discord only when it goes down or comes back. \
                                The first check only records the state.",
                "nodes": [
                    {"id": "tick", "kind": "schedule", "version": 1, "position": {"x": 0, "y": 160},
                     "config": {"every": "{{params.every}}", "catchUp": false}},
                    {"id": "check", "kind": "http.request", "version": 1, "position": {"x": 260, "y": 160},
                     "timeoutMs": 30000, "config": {"url": "{{params.url}}", "expect": "any"}},
                    {"id": "healthy", "kind": "condition", "version": 1, "position": {"x": 520, "y": 160},
                     "title": "Healthy?", "config": {"conditions": [
                        {"left": "{{steps.check.status}}", "op": "lt", "right": 400}]}},
                    {"id": "was_down", "kind": "condition", "version": 1, "position": {"x": 780, "y": 0},
                     "title": "Was it down?", "config": {"conditions": [
                        {"left": "{{state.healthy}}", "op": "eq", "right": false}]}},
                    {"id": "was_up", "kind": "condition", "version": 1, "position": {"x": 780, "y": 320},
                     "title": "Was it up?", "config": {"conditions": [
                        {"left": "{{state.healthy}}", "op": "eq", "right": true}]}},
                    {"id": "set_up", "kind": "state.set", "version": 1, "position": {"x": 1040, "y": 0},
                     "config": {"key": "healthy", "value": true}},
                    {"id": "set_down", "kind": "state.set", "version": 1, "position": {"x": 1040, "y": 320},
                     "config": {"key": "healthy", "value": false}},
                    {"id": "recovered", "kind": "discord.post", "version": 1, "position": {"x": 1300, "y": 0},
                     "retry": {"max": 3, "backoffMs": 30000}, "config": {"connection": "{{params.discord}}",
                        "content": "✅ {{params.name}} recovered."}},
                    {"id": "down", "kind": "discord.post", "version": 1, "position": {"x": 1300, "y": 320},
                     "retry": {"max": 3, "backoffMs": 30000}, "config": {"connection": "{{params.discord}}",
                        "content": "🔴 {{params.name}} is down: {{steps.check.status}}{{steps.check.error}} \
                                    ({{params.url}})"}},
                    {"id": "same_up", "kind": "state.set", "version": 1, "position": {"x": 1040, "y": 160},
                     "title": "Still up (or first check)", "config": {"key": "healthy", "value": true}},
                    {"id": "same_down", "kind": "state.set", "version": 1, "position": {"x": 1040, "y": 480},
                     "title": "Still down (or first check)", "config": {"key": "healthy", "value": false}},
                    {"id": "quiet_up", "kind": "end", "version": 1, "position": {"x": 1300, "y": 160},
                     "config": {"quiet": true}},
                    {"id": "quiet_down", "kind": "end", "version": 1, "position": {"x": 1300, "y": 480},
                     "config": {"quiet": true}},
                ],
                "edges": [
                    {"from": "tick", "to": "check"},
                    {"from": "check", "to": "healthy"},
                    {"from": "check", "port": "error", "to": "was_up"},
                    {"from": "healthy", "port": "true", "to": "was_down"},
                    {"from": "healthy", "port": "false", "to": "was_up"},
                    {"from": "was_down", "port": "true", "to": "set_up"},
                    {"from": "was_down", "port": "false", "to": "same_up"},
                    {"from": "was_up", "port": "true", "to": "set_down"},
                    {"from": "was_up", "port": "false", "to": "same_down"},
                    {"from": "set_up", "to": "recovered"},
                    {"from": "set_down", "to": "down"},
                    {"from": "same_up", "to": "quiet_up"},
                    {"from": "same_down", "to": "quiet_down"},
                ],
            })),
        ),
    ];
    for (id, name, what, every) in [
        (
            "brief-follow-ups",
            "Brief follow-ups",
            "Runs a brief's follow-up script (say, matching its items against a watchlist) once a day",
            "1d",
        ),
        ("email-processing", "Email processing", "Runs an email-processing script every 15 minutes", "15m"),
    ] {
        out.push(scheduled_script(id, name, what, every));
    }
    out
}

/// A script on a schedule that posts to Discord only when it has news: `{"message": "…"}` on stdout; `{}` or
/// `{"quiet": true}` when not.
fn scheduled_script(id: &str, name: &str, what: &str, every: &str) -> Template {
    builtin(
        id,
        params(json!([
            {"id": "program", "title": "Program", "type": "string", "default": "python3",
             "description": "What runs the script: python3, node, bash, or the script itself."},
            {"id": "script", "title": "Script", "type": "string", "required": true,
             "description": "The script's full path, given to the program. It prints {\"message\": \"…\"} when there \
                             is news and {} when not."},
            {"id": "cwd", "title": "Run in", "type": "string", "default": "scratch",
             "description": "scratch (a fresh folder), or a full path."},
            every_param(every),
            discord_param(),
        ])),
        doc(json!({
            "name": name,
            "description": format!("{what} and posts its message to Discord when it has one. The script prints \
                                    {{\"message\": \"…\"}} when there is news, {{}} when not."),
            "nodes": [
                {"id": "tick", "kind": "schedule", "version": 1, "position": {"x": 0, "y": 0},
                 "config": {"every": "{{params.every}}"}},
                {"id": "run", "kind": "command", "version": 1, "position": {"x": 260, "y": 0}, "timeoutMs": 600000,
                 "config": {"program": "{{params.program}}", "args": ["{{params.script}}"], "cwd": "{{params.cwd}}",
                    "input": {"slot": "{{trigger.slotIso}}", "date": "{{trigger.date}}"}}},
                {"id": "news", "kind": "condition", "version": 1, "position": {"x": 520, "y": 0}, "title": "Any news?",
                 "config": {"conditions": [{"left": "{{steps.run.json.message}}", "op": "truthy"}]}},
                {"id": "post", "kind": "discord.post", "version": 1, "position": {"x": 780, "y": 0},
                 "retry": {"max": 3, "backoffMs": 30000},
                 "config": {"connection": "{{params.discord}}", "content": "{{steps.run.json.message}}"}},
                {"id": "nothing", "kind": "end", "version": 1, "position": {"x": 780, "y": 160},
                 "config": {"quiet": true}},
            ],
            "edges": [
                {"from": "tick", "to": "run"},
                {"from": "run", "to": "news"},
                {"from": "news", "port": "true", "to": "post"},
                {"from": "news", "port": "false", "to": "nothing"},
            ],
        })),
    )
}
