//! The workflow templates that post to Discord (Phase 7e spec §5).

use pdt_api::sdk::templates::{builtin, doc, params};
use pdt_core::workflow::template::{Category, Template};
use serde_json::{Value, json};

/// The param of a Discord webhook connection.
fn discord_param() -> Value {
    json!({"id": "discord", "title": "Discord channel", "type": "connection", "connectionKind": "discord_webhook",
        "required": true, "description": "Where it posts: a Discord webhook connection (Settings → Connections)."})
}

/// The templates that post to Discord (Phase 7e spec §5; PDT-226 kept only the development one).
pub(crate) fn all() -> Vec<Template> {
    vec![builtin(
        "github-issues-discord",
        Category::Notifications,
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
    )]
}
